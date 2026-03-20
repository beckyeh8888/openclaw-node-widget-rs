use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::{
    AgentPlugin, ConnectionStatus, PluginCapabilities, PluginCommand, PluginError, PluginEvent,
    PluginId,
};
use crate::chat::{ChatInbound, ChatMessage, ChatSender, ChatState};
use crate::config::PluginConfig;
use crate::gateway::ChatAttachment;

// ── OpenAI API types ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct OpenAIRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIStreamChunk {
    #[serde(default)]
    pub choices: Vec<OpenAIChoice>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIChoice {
    #[serde(default)]
    pub delta: Option<OpenAIDelta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIDelta {
    #[serde(default)]
    pub content: Option<String>,
}

/// Parse SSE data from an OpenAI-compatible API into (stream_chunks, full_text).
pub fn parse_openai_sse(data: &str) -> (Vec<String>, Option<String>) {
    let mut chunks = Vec::new();
    let mut full_text = String::new();
    let mut got_done = false;

    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "data: [DONE]" {
            got_done = true;
            continue;
        }
        if let Some(json_str) = line.strip_prefix("data: ") {
            if let Ok(chunk) = serde_json::from_str::<OpenAIStreamChunk>(json_str) {
                for choice in &chunk.choices {
                    if let Some(delta) = &choice.delta {
                        if let Some(content) = &delta.content {
                            if !content.is_empty() {
                                chunks.push(content.clone());
                                full_text.push_str(content);
                            }
                        }
                    }
                }
            }
        }
    }

    if got_done {
        (chunks, Some(full_text))
    } else {
        (chunks, None)
    }
}

/// Build the authorization header value if an API key is provided.
pub fn build_auth_header(api_key: &Option<String>) -> Option<String> {
    api_key
        .as_ref()
        .filter(|k| !k.is_empty())
        .map(|k| format!("Bearer {k}"))
}

// ── Plugin ──────────────────────────────────────────────────────────

pub struct OpenAICompatPlugin {
    id: PluginId,
    plugin_name: String,
    url: String,
    model: String,
    api_key: Option<String>,
    status: Arc<Mutex<ConnectionStatus>>,
    history: Arc<Mutex<Vec<OpenAIMessage>>>,
    chat_state: Arc<Mutex<ChatState>>,
    cmd_tx: Option<mpsc::UnboundedSender<PluginCommand>>,
    event_tx: Option<mpsc::UnboundedSender<PluginEvent>>,
}

impl OpenAICompatPlugin {
    pub fn new(config: &PluginConfig, chat_state: Arc<Mutex<ChatState>>) -> Self {
        let id_str = format!("openai-{}", slug(&config.name));
        Self {
            id: PluginId(id_str),
            plugin_name: config.name.clone(),
            url: config
                .url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: config
                .model
                .clone()
                .unwrap_or_else(|| "gpt-4o".to_string()),
            api_key: config.api_key.clone(),
            status: Arc::new(Mutex::new(ConnectionStatus::Disconnected)),
            history: Arc::new(Mutex::new(Vec::new())),
            chat_state,
            cmd_tx: None,
            event_tx: None,
        }
    }

    /// Set the plugin event sender for streaming events.
    pub fn set_event_tx(&mut self, tx: mpsc::UnboundedSender<PluginEvent>) {
        self.event_tx = Some(tx);
    }

    /// Build messages array from history + new user message.
    pub fn build_messages(
        history: &[OpenAIMessage],
        user_message: &str,
    ) -> Vec<OpenAIMessage> {
        let mut messages: Vec<OpenAIMessage> = history.to_vec();
        messages.push(OpenAIMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        });
        messages
    }
}

impl AgentPlugin for OpenAICompatPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.plugin_name
    }

    fn plugin_type(&self) -> &str {
        "openai-compatible"
    }

    fn icon(&self) -> &str {
        "🤖"
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
        if self.url.is_empty() {
            return Err(PluginError("API URL not configured".to_string()));
        }

        let url = self.url.clone();
        let status = Arc::clone(&self.status);
        let id = self.id.clone();
        let api_key = self.api_key.clone();

        // Optionally verify URL with GET /models (don't fail if 404)
        let check_url = format!("{}/models", url.trim_end_matches('/'));
        let _check_result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .map_err(|e| PluginError(format!("http client error: {e}")))?;
                let mut req = client.get(&check_url);
                if let Some(auth) = build_auth_header(&api_key) {
                    req = req.header("Authorization", auth);
                }
                req.send()
                    .await
                    .map_err(|e| PluginError(format!("API unreachable at {url}: {e}")))
            })
        });

        // Don't fail on 404/network error for /models — just log
        match &_check_result {
            Ok(_) => {
                tracing::info!(plugin = %id, url = %url, "openai-compatible connected");
            }
            Err(e) => {
                tracing::warn!(plugin = %id, url = %url, error = %e, "openai-compatible /models check failed (non-fatal)");
            }
        }

        if let Ok(mut s) = status.lock() {
            *s = ConnectionStatus::Connected;
        }

        // Set up command channel
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<PluginCommand>();
        let model = self.model.clone();
        let history = Arc::clone(&self.history);
        let chat_state = Arc::clone(&self.chat_state);
        let url = self.url.clone();
        let plugin_id = self.id.clone();
        let event_tx = self.event_tx.clone();
        let api_key = self.api_key.clone();

        // Spawn command handler
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    PluginCommand::SendChat {
                        message,
                        session_key: _,
                        attachments: _,
                    } => {
                        let messages = {
                            let h = history.lock().unwrap();
                            OpenAICompatPlugin::build_messages(&h, &message)
                        };

                        let request = OpenAIRequest {
                            model: model.clone(),
                            messages,
                            stream: true,
                        };

                        let api_url = format!(
                            "{}/chat/completions",
                            url.trim_end_matches('/')
                        );

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

                        let mut req = client.post(&api_url).json(&request);
                        if let Some(auth) = build_auth_header(&api_key) {
                            req = req.header("Authorization", auth);
                        }

                        match req.send().await {
                            Ok(response) => {
                                let mut full_text = String::new();
                                let mut buf = String::new();

                                let mut resp = response;
                                loop {
                                    match resp.chunk().await {
                                        Ok(Some(bytes)) => {
                                            let text = String::from_utf8_lossy(&bytes);
                                            buf.push_str(&text);

                                            while let Some(pos) = buf.find('\n') {
                                                let line = buf[..pos].trim().to_string();
                                                buf = buf[pos + 1..].to_string();

                                                if line.is_empty() {
                                                    continue;
                                                }
                                                if line == "data: [DONE]" {
                                                    break;
                                                }
                                                if let Some(json_str) =
                                                    line.strip_prefix("data: ")
                                                {
                                                    if let Ok(chunk) =
                                                        serde_json::from_str::<
                                                            OpenAIStreamChunk,
                                                        >(
                                                            json_str
                                                        )
                                                    {
                                                        for choice in &chunk.choices {
                                                            if let Some(delta) =
                                                                &choice.delta
                                                            {
                                                                if let Some(content) =
                                                                    &delta.content
                                                                {
                                                                    if !content.is_empty() {
                                                                        full_text
                                                                            .push_str(
                                                                                content,
                                                                            );
                                                                        if let Some(tx) =
                                                                            &event_tx
                                                                        {
                                                                            let _ = tx.send(
                                                                            PluginEvent::StreamChunk { plugin_id: plugin_id.clone(), msg_id: String::new(), text: content.clone() },
                                                                        );
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Ok(None) => break,
                                        Err(e) => {
                                            tracing::error!(
                                                "openai stream error: {e}"
                                            );
                                            break;
                                        }
                                    }
                                }

                                if !full_text.is_empty() {
                                    // Update history
                                    if let Ok(mut h) = history.lock() {
                                        h.push(OpenAIMessage {
                                            role: "user".to_string(),
                                            content: message.clone(),
                                        });
                                        h.push(OpenAIMessage {
                                            role: "assistant".to_string(),
                                            content: full_text.clone(),
                                        });
                                    }

                                    // Push to chat state
                                    if let Ok(mut cs) = chat_state.lock() {
                                        cs.inbox.push(ChatInbound::Reply {
                                            text: full_text.clone(),
                                            agent_name: Some(model.clone()),
                                        });
                                        cs.waiting_for_reply = false;
                                    }

                                    // Emit MessageReceived
                                    if let Some(tx) = &event_tx {
                                        let _ = tx.send(
                                            PluginEvent::MessageReceived(
                                                plugin_id.clone(),
                                                ChatMessage {
                                                    sender: ChatSender::Agent(
                                                        model.clone(),
                                                    ),
                                                    text: full_text,
                                                },
                                            ),
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("openai request failed: {e}");
                                if let Some(tx) = &event_tx {
                                    let _ = tx.send(PluginEvent::Error(
                                        plugin_id.clone(),
                                        format!("request failed: {e}"),
                                    ));
                                }
                            }
                        }
                    }
                    PluginCommand::ListSessions => {
                        // OpenAI-compatible APIs don't have sessions
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

    #[test]
    fn parse_sse_stream_with_chunks_and_done() {
        let data = r#"data: {"choices":[{"delta":{"content":"Hello"}}]}

data: {"choices":[{"delta":{"content":" world"}}]}

data: {"choices":[{"delta":{"content":"!"}}]}

data: [DONE]
"#;
        let (chunks, full) = parse_openai_sse(data);
        assert_eq!(chunks, vec!["Hello", " world", "!"]);
        assert_eq!(full, Some("Hello world!".to_string()));
    }

    #[test]
    fn parse_sse_incomplete_stream() {
        let data = r#"data: {"choices":[{"delta":{"content":"partial"}}]}
"#;
        let (chunks, full) = parse_openai_sse(data);
        assert_eq!(chunks, vec!["partial"]);
        assert_eq!(full, None);
    }

    #[test]
    fn parse_sse_empty_delta_skipped() {
        let data = r#"data: {"choices":[{"delta":{}}]}
data: {"choices":[{"delta":{"content":"hi"}}]}
data: [DONE]
"#;
        let (chunks, full) = parse_openai_sse(data);
        assert_eq!(chunks, vec!["hi"]);
        assert_eq!(full, Some("hi".to_string()));
    }

    #[test]
    fn parse_sse_malformed_json_skipped() {
        let data = r#"data: not json
data: {"choices":[{"delta":{"content":"ok"}}]}
data: [DONE]
"#;
        let (chunks, full) = parse_openai_sse(data);
        assert_eq!(chunks, vec!["ok"]);
        assert_eq!(full, Some("ok".to_string()));
    }

    #[test]
    fn parse_sse_empty_content_skipped() {
        let data = r#"data: {"choices":[{"delta":{"content":""}}]}
data: {"choices":[{"delta":{"content":"text"}}]}
data: [DONE]
"#;
        let (chunks, full) = parse_openai_sse(data);
        assert_eq!(chunks, vec!["text"]);
        assert_eq!(full, Some("text".to_string()));
    }

    #[test]
    fn auth_header_with_key() {
        let key = Some("sk-test123".to_string());
        assert_eq!(build_auth_header(&key), Some("Bearer sk-test123".to_string()));
    }

    #[test]
    fn auth_header_without_key() {
        let key: Option<String> = None;
        assert_eq!(build_auth_header(&key), None);
    }

    #[test]
    fn auth_header_empty_key() {
        let key = Some("".to_string());
        assert_eq!(build_auth_header(&key), None);
    }

    #[test]
    fn build_messages_includes_history() {
        let history = vec![
            OpenAIMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            },
            OpenAIMessage {
                role: "assistant".to_string(),
                content: "hi".to_string(),
            },
        ];
        let messages = OpenAICompatPlugin::build_messages(&history, "how are you?");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].role, "user");
        assert_eq!(messages[2].content, "how are you?");
    }

    #[test]
    fn build_messages_empty_history() {
        let messages = OpenAICompatPlugin::build_messages(&[], "first");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn config_parsing() {
        let config = PluginConfig {
            plugin_type: "openai-compatible".to_string(),
            name: "OpenAI".to_string(),
            url: Some("https://api.openai.com/v1".to_string()),
            token: None,
            model: Some("gpt-4o".to_string()),
            api_key: Some("sk-test".to_string()),
            webhook_url: None,
            poll_url: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = OpenAICompatPlugin::new(&config, chat_state);
        assert_eq!(plugin.id().0, "openai-openai");
        assert_eq!(plugin.name(), "OpenAI");
        assert_eq!(plugin.plugin_type(), "openai-compatible");
        assert_eq!(plugin.url, "https://api.openai.com/v1");
        assert_eq!(plugin.model, "gpt-4o");
        assert_eq!(plugin.api_key, Some("sk-test".to_string()));
    }

    #[test]
    fn config_defaults() {
        let config = PluginConfig {
            plugin_type: "openai-compatible".to_string(),
            name: "Test".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: None,
            poll_url: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = OpenAICompatPlugin::new(&config, chat_state);
        assert_eq!(plugin.url, "https://api.openai.com/v1");
        assert_eq!(plugin.model, "gpt-4o");
    }

    #[test]
    fn disconnect_clears_state() {
        let config = PluginConfig {
            plugin_type: "openai-compatible".to_string(),
            name: "Test".to_string(),
            url: Some("https://api.openai.com/v1".to_string()),
            token: None,
            model: Some("gpt-4o".to_string()),
            api_key: None,
            webhook_url: None,
            poll_url: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let mut plugin = OpenAICompatPlugin::new(&config, chat_state);

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
        let config = PluginConfig {
            plugin_type: "openai-compatible".to_string(),
            name: "Test".to_string(),
            url: Some("https://api.openai.com/v1".to_string()),
            token: None,
            model: Some("gpt-4o".to_string()),
            api_key: None,
            webhook_url: None,
            poll_url: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = OpenAICompatPlugin::new(&config, chat_state);
        let result = plugin.send_message("hi", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn openai_request_serialization() {
        let req = OpenAIRequest {
            model: "gpt-4o".to_string(),
            messages: vec![OpenAIMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            stream: true,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["stream"], true);
    }
}
