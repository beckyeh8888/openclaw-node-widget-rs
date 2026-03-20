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

// ── Ollama API types ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct OllamaRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaStreamLine {
    #[serde(default)]
    pub message: Option<OllamaMessageContent>,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaMessageContent {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
}

/// Parse an NDJSON stream from Ollama into (stream_chunks, full_text).
pub fn parse_ollama_ndjson(data: &str) -> (Vec<String>, Option<String>) {
    let mut chunks = Vec::new();
    let mut full_text = String::new();
    let mut got_done = false;

    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(parsed) = serde_json::from_str::<OllamaStreamLine>(line) {
            if let Some(msg) = &parsed.message {
                if !msg.content.is_empty() {
                    chunks.push(msg.content.clone());
                    full_text.push_str(&msg.content);
                }
            }
            if parsed.done {
                got_done = true;
            }
        }
    }

    if got_done {
        (chunks, Some(full_text))
    } else {
        (chunks, None)
    }
}

// ── Plugin ──────────────────────────────────────────────────────────

pub struct OllamaPlugin {
    id: PluginId,
    plugin_name: String,
    url: String,
    model: String,
    status: Arc<Mutex<ConnectionStatus>>,
    history: Arc<Mutex<Vec<OllamaMessage>>>,
    chat_state: Arc<Mutex<ChatState>>,
    cmd_tx: Option<mpsc::UnboundedSender<PluginCommand>>,
    event_tx: Option<mpsc::UnboundedSender<PluginEvent>>,
}

impl OllamaPlugin {
    pub fn new(config: &PluginConfig, chat_state: Arc<Mutex<ChatState>>) -> Self {
        let id_str = format!("ollama-{}", slug(&config.name));
        Self {
            id: PluginId(id_str),
            plugin_name: config.name.clone(),
            url: config
                .url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            model: config
                .model
                .clone()
                .unwrap_or_else(|| "llama3.3".to_string()),
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
        history: &[OllamaMessage],
        user_message: &str,
    ) -> Vec<OllamaMessage> {
        let mut messages: Vec<OllamaMessage> = history.to_vec();
        messages.push(OllamaMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        });
        messages
    }
}

impl AgentPlugin for OllamaPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.plugin_name
    }

    fn plugin_type(&self) -> &str {
        "ollama"
    }

    fn icon(&self) -> &str {
        "🦙"
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
            return Err(PluginError("ollama URL not configured".to_string()));
        }

        let url = self.url.clone();
        let status = Arc::clone(&self.status);
        let id = self.id.clone();

        // Verify URL is reachable with GET /api/tags
        let check_url = format!("{}/api/tags", url.trim_end_matches('/'));
        let connect_result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .map_err(|e| PluginError(format!("http client error: {e}")))?;
                client
                    .get(&check_url)
                    .send()
                    .await
                    .map_err(|e| PluginError(format!("ollama unreachable at {url}: {e}")))
            })
        });

        match connect_result {
            Ok(_) => {
                tracing::info!(plugin = %id, url = %url, "ollama connected");
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
                                    OllamaPlugin::build_messages(&h, &message)
                                };

                                let request = OllamaRequest {
                                    model: model.clone(),
                                    messages,
                                    stream: true,
                                };

                                let api_url = format!(
                                    "{}/api/chat",
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

                                match client.post(&api_url).json(&request).send().await {
                                    Ok(response) => {
                                        let mut full_text = String::new();
                                        let mut buf = String::new();

                                        // Stream the response using chunk()
                                        let mut resp = response;
                                        loop {
                                            match resp.chunk().await {
                                                Ok(Some(bytes)) => {
                                                    let text =
                                                        String::from_utf8_lossy(&bytes);
                                                    buf.push_str(&text);

                                                    // Process complete lines
                                                    while let Some(pos) = buf.find('\n')
                                                    {
                                                        let line =
                                                            buf[..pos].trim().to_string();
                                                        buf = buf[pos + 1..].to_string();

                                                        if line.is_empty() {
                                                            continue;
                                                        }

                                                        if let Ok(parsed) =
                                                            serde_json::from_str::<
                                                                OllamaStreamLine,
                                                            >(
                                                                &line
                                                            )
                                                        {
                                                            if let Some(msg) =
                                                                &parsed.message
                                                            {
                                                                if !msg.content.is_empty()
                                                                {
                                                                    full_text.push_str(
                                                                        &msg.content,
                                                                    );

                                                                    // Emit stream chunk
                                                                    if let Some(tx) =
                                                                        &event_tx
                                                                    {
                                                                        let _ = tx.send(
                                                                        PluginEvent::StreamChunk {
                                                                            plugin_id: plugin_id.clone(),
                                                                            text: msg.content.clone(),
                                                                        },
                                                                    );
                                                                    }
                                                                }
                                                            }

                                                            if parsed.done {
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                                Ok(None) => break,
                                                Err(e) => {
                                                    tracing::error!(
                                                        "ollama stream error: {e}"
                                                    );
                                                    break;
                                                }
                                            }
                                        }

                                        if !full_text.is_empty() {
                                            // Update history
                                            if let Ok(mut h) = history.lock() {
                                                h.push(OllamaMessage {
                                                    role: "user".to_string(),
                                                    content: message.clone(),
                                                });
                                                h.push(OllamaMessage {
                                                    role: "assistant".to_string(),
                                                    content: full_text.clone(),
                                                });
                                            }

                                            // Push to chat state
                                            if let Ok(mut cs) = chat_state.lock() {
                                                cs.inbox.push(ChatInbound::Reply {
                                                    text: full_text.clone(),
                                                    agent_name: Some(
                                                        model.clone(),
                                                    ),
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
                                        tracing::error!("ollama request failed: {e}");
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
                                // Ollama doesn't have sessions
                            }
                        }
                    }
                });

                self.cmd_tx = Some(cmd_tx);
                Ok(())
            }
            Err(e) => {
                if let Ok(mut s) = status.lock() {
                    *s = ConnectionStatus::Error(e.0.clone());
                }
                Err(e)
            }
        }
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
        // Ollama doesn't have sessions
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
    fn parse_ndjson_stream_with_chunks_and_done() {
        let data = r#"{"message":{"role":"assistant","content":"Hello"},"done":false}
{"message":{"role":"assistant","content":" world"},"done":false}
{"message":{"role":"assistant","content":"!"},"done":false}
{"message":{"role":"assistant","content":""},"done":true}
"#;
        let (chunks, full) = parse_ollama_ndjson(data);
        assert_eq!(chunks, vec!["Hello", " world", "!"]);
        assert_eq!(full, Some("Hello world!".to_string()));
    }

    #[test]
    fn parse_ndjson_incomplete_stream() {
        let data = r#"{"message":{"role":"assistant","content":"partial"},"done":false}
"#;
        let (chunks, full) = parse_ollama_ndjson(data);
        assert_eq!(chunks, vec!["partial"]);
        assert_eq!(full, None);
    }

    #[test]
    fn parse_ndjson_empty_content_skipped() {
        let data = r#"{"message":{"role":"assistant","content":""},"done":false}
{"message":{"role":"assistant","content":"hi"},"done":false}
{"done":true}
"#;
        let (chunks, full) = parse_ollama_ndjson(data);
        assert_eq!(chunks, vec!["hi"]);
        assert_eq!(full, Some("hi".to_string()));
    }

    #[test]
    fn parse_ndjson_malformed_line_skipped() {
        let data = r#"not json
{"message":{"role":"assistant","content":"ok"},"done":false}
{"done":true}
"#;
        let (chunks, full) = parse_ollama_ndjson(data);
        assert_eq!(chunks, vec!["ok"]);
        assert_eq!(full, Some("ok".to_string()));
    }

    #[test]
    fn build_messages_includes_history() {
        let history = vec![
            OllamaMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            },
            OllamaMessage {
                role: "assistant".to_string(),
                content: "hi".to_string(),
            },
        ];
        let messages = OllamaPlugin::build_messages(&history, "how are you?");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "hi");
        assert_eq!(messages[2].role, "user");
        assert_eq!(messages[2].content, "how are you?");
    }

    #[test]
    fn build_messages_empty_history() {
        let messages = OllamaPlugin::build_messages(&[], "first message");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "first message");
    }

    #[test]
    fn config_parsing() {
        let config = PluginConfig {
            plugin_type: "ollama".to_string(),
            name: "Local Llama".to_string(),
            url: Some("http://localhost:11434".to_string()),
            token: None,
            model: Some("llama3.3".to_string()),
            api_key: None,
            webhook_url: None,
            poll_url: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = OllamaPlugin::new(&config, chat_state);
        assert_eq!(plugin.id().0, "ollama-local-llama");
        assert_eq!(plugin.name(), "Local Llama");
        assert_eq!(plugin.plugin_type(), "ollama");
        assert_eq!(plugin.url, "http://localhost:11434");
        assert_eq!(plugin.model, "llama3.3");
    }

    #[test]
    fn config_defaults() {
        let config = PluginConfig {
            plugin_type: "ollama".to_string(),
            name: "Test".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: None,
            poll_url: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = OllamaPlugin::new(&config, chat_state);
        assert_eq!(plugin.url, "http://localhost:11434");
        assert_eq!(plugin.model, "llama3.3");
    }

    #[test]
    fn disconnect_clears_state() {
        let config = PluginConfig {
            plugin_type: "ollama".to_string(),
            name: "Test".to_string(),
            url: Some("http://localhost:11434".to_string()),
            token: None,
            model: Some("llama3.3".to_string()),
            api_key: None,
            webhook_url: None,
            poll_url: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let mut plugin = OllamaPlugin::new(&config, chat_state);

        // Manually set connected state
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
            plugin_type: "ollama".to_string(),
            name: "Test".to_string(),
            url: Some("http://localhost:11434".to_string()),
            token: None,
            model: Some("llama3.3".to_string()),
            api_key: None,
            webhook_url: None,
            poll_url: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = OllamaPlugin::new(&config, chat_state);
        let result = plugin.send_message("hi", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn ollama_request_serialization() {
        let req = OllamaRequest {
            model: "llama3.3".to_string(),
            messages: vec![OllamaMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            stream: true,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "llama3.3");
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "hi");
        assert_eq!(json["stream"], true);
    }
}
