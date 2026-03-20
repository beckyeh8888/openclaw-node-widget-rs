use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::{
    AgentPlugin, ConnectionStatus, HealthStatus, PluginCapabilities, PluginCommand, PluginError,
    PluginEvent, PluginId,
};
use crate::chat::{ChatInbound, ChatMessage, ChatSender, ChatState};
use crate::config::PluginConfig;
use crate::gateway::ChatAttachment;

// ── MCP JSON-RPC types ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    #[allow(dead_code)]
    pub jsonrpc: String,
    #[allow(dead_code)]
    pub id: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpMessage {
    pub role: String,
    pub content: McpContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

/// Build the `initialize` JSON-RPC request.
pub fn build_initialize_request(id: u64) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id,
        method: "initialize".to_string(),
        params: Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "openclaw-widget",
                "version": "0.9.0"
            }
        })),
    }
}

/// Build a `sampling/createMessage` JSON-RPC request.
pub fn build_create_message_request(id: u64, messages: &[McpMessage], max_tokens: u64) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id,
        method: "sampling/createMessage".to_string(),
        params: Some(serde_json::json!({
            "messages": messages,
            "maxTokens": max_tokens,
        })),
    }
}

/// Parse an assistant response from a JSON-RPC result.
pub fn parse_assistant_response(result: &serde_json::Value) -> Option<String> {
    // Try content.text first (standard MCP response)
    if let Some(content) = result.get("content") {
        if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
            return Some(text.to_string());
        }
    }
    // Try top-level text
    if let Some(text) = result.get("text").and_then(|t| t.as_str()) {
        return Some(text.to_string());
    }
    // Try model field with content
    if let Some(model) = result.get("model") {
        if model.is_string() {
            if let Some(content) = result.get("content") {
                if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

// ── Transport ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
    },
    Sse {
        url: String,
    },
}

// ── Plugin ─────────────────────────────────────────────────────────

pub struct McpPlugin {
    id: PluginId,
    plugin_name: String,
    transport: McpTransport,
    status: Arc<Mutex<ConnectionStatus>>,
    messages: Arc<Mutex<Vec<McpMessage>>>,
    chat_state: Arc<Mutex<ChatState>>,
    cmd_tx: Option<mpsc::UnboundedSender<PluginCommand>>,
    event_tx: Option<mpsc::UnboundedSender<PluginEvent>>,
}

impl McpPlugin {
    pub fn new(config: &PluginConfig, chat_state: Arc<Mutex<ChatState>>) -> Self {
        let id_str = format!("mcp-{}", slug(&config.name));
        let transport_type = config.transport.as_deref().unwrap_or("stdio");

        let transport = match transport_type {
            "sse" => McpTransport::Sse {
                url: config.url.clone().unwrap_or_default(),
            },
            _ => McpTransport::Stdio {
                command: config.command.clone().unwrap_or_default(),
                args: config.args.clone().unwrap_or_default(),
            },
        };

        Self {
            id: PluginId(id_str),
            plugin_name: config.name.clone(),
            transport,
            status: Arc::new(Mutex::new(ConnectionStatus::Disconnected)),
            messages: Arc::new(Mutex::new(Vec::new())),
            chat_state,
            cmd_tx: None,
            event_tx: None,
        }
    }

    pub fn set_event_tx(&mut self, tx: mpsc::UnboundedSender<PluginEvent>) {
        self.event_tx = Some(tx);
    }

    pub fn transport(&self) -> &McpTransport {
        &self.transport
    }
}

impl AgentPlugin for McpPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.plugin_name
    }

    fn plugin_type(&self) -> &str {
        "mcp"
    }

    fn icon(&self) -> &str {
        "🔌"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities {
            chat: true,
            dashboard: false,
            workflows: false,
            logs: false,
        }
    }

    fn status(&self) -> ConnectionStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(ConnectionStatus::Disconnected)
    }

    fn connect(&mut self) -> Result<(), PluginError> {
        match &self.transport {
            McpTransport::Stdio { command, .. } => {
                if command.is_empty() {
                    return Err(PluginError("MCP stdio command not configured".to_string()));
                }
            }
            McpTransport::Sse { url } => {
                if url.is_empty() {
                    return Err(PluginError("MCP SSE url not configured".to_string()));
                }
            }
        }

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<PluginCommand>();
        let status = Arc::clone(&self.status);
        let messages = Arc::clone(&self.messages);
        let chat_state = Arc::clone(&self.chat_state);
        let plugin_id = self.id.clone();
        let event_tx = self.event_tx.clone();
        let transport = self.transport.clone();

        if let Ok(mut s) = status.lock() {
            *s = ConnectionStatus::Connected;
        }

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    PluginCommand::SendChat {
                        message,
                        session_key: _,
                        attachments: _,
                    } => {
                        // Add user message to history
                        let mcp_messages = {
                            let mut msgs = messages.lock().unwrap();
                            msgs.push(McpMessage {
                                role: "user".to_string(),
                                content: McpContent {
                                    content_type: "text".to_string(),
                                    text: message.clone(),
                                },
                            });
                            msgs.clone()
                        };

                        let request = build_create_message_request(2, &mcp_messages, 4096);
                        let request_json = match serde_json::to_string(&request) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!("failed to serialize MCP request: {e}");
                                continue;
                            }
                        };

                        let response_text = match &transport {
                            McpTransport::Stdio { command, args } => {
                                // Send request via stdin to child process
                                let spawn_result = {
                                    let mut cmd = tokio::process::Command::new(command);
                                    cmd.args(args)
                                        .stdin(std::process::Stdio::piped())
                                        .stdout(std::process::Stdio::piped())
                                        .stderr(std::process::Stdio::null());
                                    #[cfg(windows)]
                                    {
                                        use std::os::windows::process::CommandExt;
                                        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                                    }
                                    cmd.spawn()
                                };
                                match spawn_result {
                                    Ok(mut child) => {
                                        // Write init + request
                                        if let Some(mut stdin) = child.stdin.take() {
                                            use tokio::io::AsyncWriteExt;
                                            let init = build_initialize_request(1);
                                            let init_json = serde_json::to_string(&init).unwrap();
                                            let _ = stdin.write_all(init_json.as_bytes()).await;
                                            let _ = stdin.write_all(b"\n").await;
                                            let _ = stdin.write_all(request_json.as_bytes()).await;
                                            let _ = stdin.write_all(b"\n").await;
                                            drop(stdin);
                                        }

                                        match child.wait_with_output().await {
                                            Ok(output) => {
                                                String::from_utf8_lossy(&output.stdout).to_string()
                                            }
                                            Err(e) => {
                                                tracing::error!("MCP stdio error: {e}");
                                                if let Some(tx) = &event_tx {
                                                    let _ = tx.send(PluginEvent::Error(
                                                        plugin_id.clone(),
                                                        format!("stdio error: {e}"),
                                                    ));
                                                }
                                                continue;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("MCP spawn error: {e}");
                                        if let Some(tx) = &event_tx {
                                            let _ = tx.send(PluginEvent::Error(
                                                plugin_id.clone(),
                                                format!("spawn error: {e}"),
                                            ));
                                        }
                                        continue;
                                    }
                                }
                            }
                            McpTransport::Sse { url } => {
                                let client = match reqwest::Client::builder()
                                    .timeout(std::time::Duration::from_secs(120))
                                    .build()
                                {
                                    Ok(c) => c,
                                    Err(e) => {
                                        tracing::error!("http client error: {e}");
                                        continue;
                                    }
                                };

                                match client
                                    .post(url)
                                    .header("Content-Type", "application/json")
                                    .body(request_json)
                                    .send()
                                    .await
                                {
                                    Ok(resp) => match resp.text().await {
                                        Ok(t) => t,
                                        Err(e) => {
                                            tracing::error!("MCP SSE read error: {e}");
                                            continue;
                                        }
                                    },
                                    Err(e) => {
                                        tracing::error!("MCP SSE request error: {e}");
                                        if let Some(tx) = &event_tx {
                                            let _ = tx.send(PluginEvent::Error(
                                                plugin_id.clone(),
                                                format!("SSE error: {e}"),
                                            ));
                                        }
                                        continue;
                                    }
                                }
                            }
                        };

                        // Parse the last JSON-RPC response line (skip init response)
                        let mut assistant_text = None;
                        for line in response_text.lines().rev() {
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(line) {
                                if let Some(err) = &resp.error {
                                    tracing::error!(
                                        "MCP error {}: {}",
                                        err.code,
                                        err.message
                                    );
                                    if let Some(tx) = &event_tx {
                                        let _ = tx.send(PluginEvent::Error(
                                            plugin_id.clone(),
                                            format!("MCP error: {}", err.message),
                                        ));
                                    }
                                    break;
                                }
                                if let Some(result) = &resp.result {
                                    if let Some(text) = parse_assistant_response(result) {
                                        assistant_text = Some(text);
                                        break;
                                    }
                                }
                            }
                        }

                        if let Some(text) = assistant_text {
                            // Update message history
                            if let Ok(mut msgs) = messages.lock() {
                                msgs.push(McpMessage {
                                    role: "assistant".to_string(),
                                    content: McpContent {
                                        content_type: "text".to_string(),
                                        text: text.clone(),
                                    },
                                });
                            }

                            // Push to chat state
                            if let Ok(mut cs) = chat_state.lock() {
                                cs.inbox.push(ChatInbound::Reply {
                                    text: text.clone(),
                                    agent_name: Some("MCP".to_string()),
                                    usage: None,
                                });
                                cs.waiting_for_reply = false;
                            }

                            // Emit event
                            if let Some(tx) = &event_tx {
                                let _ = tx.send(PluginEvent::MessageReceived(
                                    plugin_id.clone(),
                                    ChatMessage {
                                        sender: ChatSender::Agent("MCP".to_string()),
                                        text,
                                    },
                                    None,
                                ));
                            }
                        }
                    }
                    PluginCommand::ListSessions => {
                        // MCP doesn't have sessions
                    }
                }
            }
        });

        self.cmd_tx = Some(cmd_tx);
        Ok(())
    }

    fn disconnect(&mut self) -> Result<(), PluginError> {
        self.cmd_tx = None;
        if let Ok(mut s) = self.status.lock() {
            *s = ConnectionStatus::Disconnected;
        }
        Ok(())
    }

    fn send_message(
        &self,
        message: &str,
        session_key: Option<String>,
        attachments: Option<Vec<ChatAttachment>>,
    ) -> Result<(), PluginError> {
        let tx = self
            .cmd_tx
            .as_ref()
            .ok_or_else(|| PluginError("not connected".to_string()))?;
        tx.send(PluginCommand::SendChat {
            message: message.to_string(),
            session_key,
            attachments,
        })
        .map_err(|e| PluginError(format!("send failed: {e}")))?;
        Ok(())
    }

    fn list_sessions(&self) -> Result<(), PluginError> {
        Ok(())
    }

    fn command_sender(&self) -> Option<mpsc::UnboundedSender<PluginCommand>> {
        self.cmd_tx.clone()
    }

    fn health_check(&self) -> HealthStatus {
        match &self.transport {
            McpTransport::Sse { url } => {
                if url.is_empty() {
                    return HealthStatus {
                        reachable: false,
                        latency_ms: 0,
                        error: Some("SSE URL not configured".to_string()),
                    };
                }
                let start = std::time::Instant::now();
                let client = match reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                {
                    Ok(c) => c,
                    Err(e) => return HealthStatus { reachable: false, latency_ms: 0, error: Some(format!("{e}")) },
                };
                let result = client.head(url).send();
                let latency_ms = start.elapsed().as_millis() as u64;
                match result {
                    Ok(_) => HealthStatus { reachable: true, latency_ms, error: None },
                    Err(e) => HealthStatus { reachable: false, latency_ms, error: Some(format!("{e}")) },
                }
            }
            McpTransport::Stdio { command, .. } => {
                // For stdio, just check the command binary exists
                let reachable = !command.is_empty() && {
                    let mut cmd = std::process::Command::new(command);
                    cmd.arg("--version")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null());
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                    }
                    cmd.status().is_ok()
                };
                HealthStatus {
                    reachable,
                    latency_ms: 0,
                    error: if reachable {
                        None
                    } else {
                        Some(format!("command '{}' not found", command))
                    },
                }
            }
        }
    }
}

fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(transport: &str) -> PluginConfig {
        PluginConfig {
            plugin_type: "mcp".to_string(),
            name: "File Browser".to_string(),
            url: Some("http://localhost:3001".to_string()),
            token: None,
            model: None,
            api_key: None,
            webhook_url: None,
            poll_url: None,
            transport: Some(transport.to_string()),
            command: Some("npx".to_string()),
            args: Some(vec![
                "-y".to_string(),
                "@anthropic/mcp-server-filesystem".to_string(),
                "/home".to_string(),
            ]),
            system_prompt: None,
        }
    }

    #[test]
    fn config_parsing_stdio() {
        let config = make_config("stdio");
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = McpPlugin::new(&config, chat_state);

        assert_eq!(plugin.id().0, "mcp-file-browser");
        assert_eq!(plugin.name(), "File Browser");
        assert_eq!(plugin.plugin_type(), "mcp");
        match plugin.transport() {
            McpTransport::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert_eq!(args.len(), 3);
            }
            _ => panic!("expected Stdio transport"),
        }
    }

    #[test]
    fn config_parsing_sse() {
        let config = make_config("sse");
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = McpPlugin::new(&config, chat_state);

        match plugin.transport() {
            McpTransport::Sse { url } => {
                assert_eq!(url, "http://localhost:3001");
            }
            _ => panic!("expected SSE transport"),
        }
    }

    #[test]
    fn initialize_request_format() {
        let req = build_initialize_request(1);
        let json = serde_json::to_value(&req).unwrap();

        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "initialize");
        assert_eq!(json["params"]["protocolVersion"], "2024-11-05");
        assert_eq!(json["params"]["clientInfo"]["name"], "openclaw-widget");
    }

    #[test]
    fn create_message_request_format() {
        let messages = vec![McpMessage {
            role: "user".to_string(),
            content: McpContent {
                content_type: "text".to_string(),
                text: "Hello".to_string(),
            },
        }];
        let req = build_create_message_request(2, &messages, 4096);
        let json = serde_json::to_value(&req).unwrap();

        assert_eq!(json["method"], "sampling/createMessage");
        assert_eq!(json["params"]["messages"][0]["role"], "user");
        assert_eq!(json["params"]["messages"][0]["content"]["text"], "Hello");
        assert_eq!(json["params"]["maxTokens"], 4096);
    }

    #[test]
    fn parse_assistant_response_from_content() {
        let result = serde_json::json!({
            "content": { "type": "text", "text": "Hello from MCP!" }
        });
        assert_eq!(
            parse_assistant_response(&result),
            Some("Hello from MCP!".to_string())
        );
    }

    #[test]
    fn parse_assistant_response_from_text() {
        let result = serde_json::json!({ "text": "fallback text" });
        assert_eq!(
            parse_assistant_response(&result),
            Some("fallback text".to_string())
        );
    }

    #[test]
    fn parse_assistant_response_none_on_empty() {
        let result = serde_json::json!({});
        assert_eq!(parse_assistant_response(&result), None);
    }

    #[test]
    fn handle_error_response() {
        let resp_json = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32600,"message":"Invalid request"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(resp_json).unwrap();
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "Invalid request");
    }

    #[test]
    fn capabilities_chat_only() {
        let config = make_config("stdio");
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = McpPlugin::new(&config, chat_state);
        let caps = plugin.capabilities();
        assert!(caps.chat);
        assert!(!caps.dashboard);
        assert!(!caps.workflows);
        assert!(!caps.logs);
    }

    #[test]
    fn disconnect_clears_state() {
        let config = make_config("stdio");
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let mut plugin = McpPlugin::new(&config, chat_state);

        {
            let mut s = plugin.status.lock().unwrap();
            *s = ConnectionStatus::Connected;
        }
        let (tx, _rx) = mpsc::unbounded_channel();
        plugin.cmd_tx = Some(tx);

        plugin.disconnect().unwrap();
        assert_eq!(plugin.status(), ConnectionStatus::Disconnected);
        assert!(plugin.cmd_tx.is_none());
    }

    #[test]
    fn send_message_without_connect_fails() {
        let config = make_config("stdio");
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = McpPlugin::new(&config, chat_state);
        let result = plugin.send_message("hi", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn connect_fails_without_command() {
        let config = PluginConfig {
            plugin_type: "mcp".to_string(),
            name: "Bad".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: None,
            poll_url: None,
            transport: Some("stdio".to_string()),
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let mut plugin = McpPlugin::new(&config, chat_state);
        let result = plugin.connect();
        assert!(result.is_err());
        assert!(result.unwrap_err().0.contains("command not configured"));
    }

    #[test]
    fn connect_sse_fails_without_url() {
        let config = PluginConfig {
            plugin_type: "mcp".to_string(),
            name: "Bad SSE".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: None,
            poll_url: None,
            transport: Some("sse".to_string()),
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let mut plugin = McpPlugin::new(&config, chat_state);
        let result = plugin.connect();
        assert!(result.is_err());
        assert!(result.unwrap_err().0.contains("url not configured"));
    }
}
