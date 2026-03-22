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
use crate::i18n::t;

// ── n8n API types ────────────────────────────────────────────────────

/// A single chat history entry sent to the n8n webhook.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub role: String,
    pub content: String,
}

const MAX_HISTORY_ENTRIES: usize = 20;

#[derive(Debug, Clone, Serialize)]
pub struct N8nRequest {
    pub message: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub history: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct N8nResponse {
    #[serde(default)]
    pub response: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

impl N8nResponse {
    /// Extract the response text from whichever field n8n provided.
    pub fn text(&self) -> Option<&str> {
        self.response
            .as_deref()
            .or(self.output.as_deref())
            .or(self.text.as_deref())
    }
}

/// Parse an n8n webhook JSON response body into its text content.
pub fn parse_n8n_response(body: &str) -> Option<String> {
    // Try object first
    if let Ok(resp) = serde_json::from_str::<N8nResponse>(body) {
        if let Some(t) = resp.text() {
            return Some(t.to_string());
        }
    }
    // Try array of objects (some n8n workflows return an array)
    if let Ok(arr) = serde_json::from_str::<Vec<N8nResponse>>(body) {
        for item in &arr {
            if let Some(t) = item.text() {
                return Some(t.to_string());
            }
        }
    }
    None
}

// ── Default constants ────────────────────────────────────────────────

const DEFAULT_POLL_INTERVAL_MS: u64 = 2000;
const MAX_POLL_TIMEOUT_SECS: u64 = 60;

// ── Plugin ───────────────────────────────────────────────────────────

pub struct N8nPlugin {
    id: PluginId,
    plugin_name: String,
    webhook_url: String,
    poll_url: Option<String>,
    poll_interval_ms: u64,
    status: Arc<Mutex<ConnectionStatus>>,
    chat_state: Arc<Mutex<ChatState>>,
    history: Arc<Mutex<Vec<HistoryEntry>>>,
    cmd_tx: Option<mpsc::UnboundedSender<PluginCommand>>,
    event_tx: Option<mpsc::UnboundedSender<PluginEvent>>,
}

impl N8nPlugin {
    pub fn new(config: &PluginConfig, chat_state: Arc<Mutex<ChatState>>) -> Self {
        let id_str = format!("n8n-{}", slug(&config.name));
        Self {
            id: PluginId(id_str),
            plugin_name: config.name.clone(),
            webhook_url: config
                .webhook_url
                .clone()
                .or_else(|| config.url.clone())
                .unwrap_or_default(),
            poll_url: config.poll_url.clone(),
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
            status: Arc::new(Mutex::new(ConnectionStatus::Disconnected)),
            chat_state,
            history: Arc::new(Mutex::new(Vec::new())),
            cmd_tx: None,
            event_tx: None,
        }
    }

    /// Set the plugin event sender for events.
    pub fn set_event_tx(&mut self, tx: mpsc::UnboundedSender<PluginEvent>) {
        self.event_tx = Some(tx);
    }
}

impl AgentPlugin for N8nPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.plugin_name
    }

    fn plugin_type(&self) -> &str {
        "n8n"
    }

    fn icon(&self) -> &str {
        "⚡"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities {
            chat: true,
            dashboard: false,
            workflows: true,
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
        if self.webhook_url.is_empty() {
            return Err(PluginError("n8n webhook_url not configured".to_string()));
        }

        let url = self.webhook_url.clone();
        let status = Arc::clone(&self.status);
        let id = self.id.clone();

        // Verify webhook URL is reachable with HEAD
        let connect_result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .map_err(|e| PluginError(format!("http client error: {e}")))?;
                client
                    .head(&url)
                    .send()
                    .await
                    .map_err(|e| PluginError(format!("n8n webhook unreachable at {url}: {e}")))
            })
        });

        match connect_result {
            Ok(_) => {
                tracing::info!(plugin = %id, url = %url, "n8n connected");
                if let Ok(mut s) = status.lock() {
                    *s = ConnectionStatus::Connected;
                }

                // Set up command channel
                let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<PluginCommand>();
                let webhook_url = self.webhook_url.clone();
                let poll_url = self.poll_url.clone();
                let poll_interval_ms = self.poll_interval_ms;
                let chat_state = Arc::clone(&self.chat_state);
                let history = Arc::clone(&self.history);
                let plugin_id = self.id.clone();
                let event_tx = self.event_tx.clone();
                let plugin_name = self.plugin_name.clone();

                // Spawn command handler
                tokio::spawn(async move {
                    while let Some(cmd) = cmd_rx.recv().await {
                        match cmd {
                            PluginCommand::SendChat {
                                message,
                                session_key: _,
                                attachments: _,
                            } => {
                                let current_history = history
                                    .lock()
                                    .map(|h| h.clone())
                                    .unwrap_or_default();
                                let request = N8nRequest {
                                    message: message.clone(),
                                    session_id: plugin_id.0.clone(),
                                    history: current_history,
                                };

                                let client = match reqwest::Client::builder()
                                    .timeout(std::time::Duration::from_secs(
                                        MAX_POLL_TIMEOUT_SECS + 10,
                                    ))
                                    .build()
                                {
                                    Ok(c) => c,
                                    Err(e) => {
                                        tracing::error!("http client error: {e}");
                                        continue;
                                    }
                                };

                                match client
                                    .post(&webhook_url)
                                    .json(&request)
                                    .send()
                                    .await
                                {
                                    Ok(response) => {
                                        let status_code = response.status();
                                        let body = response
                                            .text()
                                            .await
                                            .unwrap_or_default();

                                        if status_code.as_u16() == 202 {
                                            // Async: poll for result
                                            if let Some(ref purl) = poll_url {
                                                let result = poll_for_result(
                                                    &client,
                                                    purl,
                                                    poll_interval_ms,
                                                    MAX_POLL_TIMEOUT_SECS,
                                                )
                                                .await;
                                                match result {
                                                    Some(text) => {
                                                        emit_reply(
                                                            &text,
                                                            &plugin_name,
                                                            &plugin_id,
                                                            &chat_state,
                                                            &event_tx,
                                                        );
                                                        record_exchange(
                                                            &history,
                                                            &message,
                                                            &text,
                                                        );
                                                    }
                                                    None => {
                                                        tracing::warn!(
                                                            "n8n poll timed out after {MAX_POLL_TIMEOUT_SECS}s"
                                                        );
                                                        if let Some(tx) = &event_tx {
                                                            let _ = tx.send(
                                                                PluginEvent::Error(
                                                                    plugin_id.clone(),
                                                                    "poll timed out".to_string(),
                                                                ),
                                                            );
                                                        }
                                                    }
                                                }
                                            } else {
                                                tracing::warn!("n8n returned 202 but no poll_url configured");
                                            }
                                        } else if status_code.is_success() {
                                            // Immediate response
                                            if let Some(text) =
                                                parse_n8n_response(&body)
                                            {
                                                emit_reply(
                                                    &text,
                                                    &plugin_name,
                                                    &plugin_id,
                                                    &chat_state,
                                                    &event_tx,
                                                );
                                                record_exchange(
                                                    &history,
                                                    &message,
                                                    &text,
                                                );
                                            } else if !body.is_empty() {
                                                // Fallback: use raw body
                                                emit_reply(
                                                    &body,
                                                    &plugin_name,
                                                    &plugin_id,
                                                    &chat_state,
                                                    &event_tx,
                                                );
                                                record_exchange(
                                                    &history,
                                                    &message,
                                                    &body,
                                                );
                                            }
                                        } else {
                                            tracing::error!(
                                                "n8n webhook returned {status_code}: {body}"
                                            );
                                            if let Some(tx) = &event_tx {
                                                let _ = tx.send(PluginEvent::Error(
                                                    plugin_id.clone(),
                                                    format!(
                                                        "webhook error: {status_code}"
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("n8n request failed: {e}");
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
                                // n8n doesn't have sessions
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
        // Handle /clear command locally without calling webhook
        if message.trim().eq_ignore_ascii_case("/clear") {
            if let Ok(mut h) = self.history.lock() {
                h.clear();
            }
            emit_reply(
                t("chat_history_cleared"),
                &self.plugin_name,
                &self.id,
                &self.chat_state,
                &self.event_tx,
            );
            return Ok(());
        }

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
        // n8n doesn't have sessions
        Ok(())
    }

    fn command_sender(&self) -> Option<mpsc::UnboundedSender<PluginCommand>> {
        self.cmd_tx.clone()
    }

    fn health_check(&self) -> HealthStatus {
        if self.webhook_url.is_empty() {
            return HealthStatus {
                reachable: false,
                latency_ms: 0,
                error: Some("webhook URL not configured".to_string()),
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
        let result = client.head(&self.webhook_url).send();
        let latency_ms = start.elapsed().as_millis() as u64;
        match result {
            Ok(_) => HealthStatus { reachable: true, latency_ms, error: None },
            Err(e) => HealthStatus { reachable: false, latency_ms, error: Some(format!("{e}")) },
        }
    }
}

/// Record a user–assistant exchange in history, capping at MAX_HISTORY_ENTRIES.
fn record_exchange(
    history: &Arc<Mutex<Vec<HistoryEntry>>>,
    user_msg: &str,
    assistant_msg: &str,
) {
    if let Ok(mut h) = history.lock() {
        h.push(HistoryEntry {
            role: "user".to_string(),
            content: user_msg.to_string(),
        });
        h.push(HistoryEntry {
            role: "assistant".to_string(),
            content: assistant_msg.to_string(),
        });
        if h.len() > MAX_HISTORY_ENTRIES {
            let excess = h.len() - MAX_HISTORY_ENTRIES;
            h.drain(..excess);
        }
    }
}

/// Push a reply into the chat state and emit a MessageReceived event.
fn emit_reply(
    text: &str,
    plugin_name: &str,
    plugin_id: &PluginId,
    chat_state: &Arc<Mutex<ChatState>>,
    event_tx: &Option<mpsc::UnboundedSender<PluginEvent>>,
) {
    if let Ok(mut cs) = chat_state.lock() {
        cs.inbox.push(ChatInbound::Reply {
            text: text.to_string(),
            agent_name: Some(plugin_name.to_string()),
            usage: None,
            attachments: None,
        });
        cs.waiting_for_reply = false;
    }
    if let Some(tx) = event_tx {
        let _ = tx.send(PluginEvent::MessageReceived(
            plugin_id.clone(),
            ChatMessage {
                sender: ChatSender::Agent(plugin_name.to_string()),
                text: text.to_string(),
                media_path: None,
                media_type: None,
            },
            None,
        ));
    }
}

/// Poll a URL until a response with text is returned or timeout.
async fn poll_for_result(
    client: &reqwest::Client,
    poll_url: &str,
    interval_ms: u64,
    timeout_secs: u64,
) -> Option<String> {
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let interval = std::time::Duration::from_millis(interval_ms);

    loop {
        if tokio::time::Instant::now() >= deadline {
            return None;
        }

        tokio::time::sleep(interval).await;

        match client.get(poll_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    if let Some(text) = parse_n8n_response(&body) {
                        return Some(text);
                    }
                }
            }
            _ => {}
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

    #[test]
    fn parse_response_object_with_response_field() {
        let body = r#"{"response":"Hello from n8n!"}"#;
        assert_eq!(parse_n8n_response(body), Some("Hello from n8n!".to_string()));
    }

    #[test]
    fn parse_response_object_with_output_field() {
        let body = r#"{"output":"workflow result"}"#;
        assert_eq!(parse_n8n_response(body), Some("workflow result".to_string()));
    }

    #[test]
    fn parse_response_object_with_text_field() {
        let body = r#"{"text":"hello"}"#;
        assert_eq!(parse_n8n_response(body), Some("hello".to_string()));
    }

    #[test]
    fn parse_response_array() {
        let body = r#"[{"response":"first"}]"#;
        assert_eq!(parse_n8n_response(body), Some("first".to_string()));
    }

    #[test]
    fn parse_response_empty_object() {
        let body = r#"{}"#;
        assert_eq!(parse_n8n_response(body), None);
    }

    #[test]
    fn parse_response_invalid_json() {
        let body = "not json";
        assert_eq!(parse_n8n_response(body), None);
    }

    #[test]
    fn n8n_request_serialization() {
        let req = N8nRequest {
            message: "hello".to_string(),
            session_id: "widget".to_string(),
            history: vec![],
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["message"], "hello");
        assert_eq!(json["sessionId"], "widget");
        assert_eq!(json["history"], serde_json::json!([]));
    }

    #[test]
    fn n8n_request_serialization_with_history() {
        let req = N8nRequest {
            message: "next".to_string(),
            session_id: "n8n-test".to_string(),
            history: vec![
                HistoryEntry { role: "user".into(), content: "hi".into() },
                HistoryEntry { role: "assistant".into(), content: "hello".into() },
            ],
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["history"][0]["role"], "user");
        assert_eq!(json["history"][0]["content"], "hi");
        assert_eq!(json["history"][1]["role"], "assistant");
        assert_eq!(json["history"][1]["content"], "hello");
    }

    #[test]
    fn config_parsing_webhook_url() {
        let config = PluginConfig {
            plugin_type: "n8n".to_string(),
            name: "My Workflow".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: Some("https://n8n.example.com/webhook/abc".to_string()),
            poll_url: Some("https://n8n.example.com/webhook/abc/poll".to_string()),
            transport: None,
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = N8nPlugin::new(&config, chat_state);
        assert_eq!(plugin.id().0, "n8n-my-workflow");
        assert_eq!(plugin.name(), "My Workflow");
        assert_eq!(plugin.plugin_type(), "n8n");
        assert_eq!(plugin.webhook_url, "https://n8n.example.com/webhook/abc");
        assert_eq!(
            plugin.poll_url,
            Some("https://n8n.example.com/webhook/abc/poll".to_string())
        );
    }

    #[test]
    fn config_falls_back_to_url_field() {
        let config = PluginConfig {
            plugin_type: "n8n".to_string(),
            name: "Test".to_string(),
            url: Some("https://n8n.example.com/webhook/fallback".to_string()),
            token: None,
            model: None,
            api_key: None,
            webhook_url: None,
            poll_url: None,
            transport: None,
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = N8nPlugin::new(&config, chat_state);
        assert_eq!(plugin.webhook_url, "https://n8n.example.com/webhook/fallback");
        assert_eq!(plugin.poll_url, None);
    }

    #[test]
    fn disconnect_clears_state() {
        let config = PluginConfig {
            plugin_type: "n8n".to_string(),
            name: "Test".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: Some("https://n8n.example.com/webhook/abc".to_string()),
            poll_url: None,
            transport: None,
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let mut plugin = N8nPlugin::new(&config, chat_state);

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
            plugin_type: "n8n".to_string(),
            name: "Test".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: Some("https://n8n.example.com/webhook/abc".to_string()),
            poll_url: None,
            transport: None,
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = N8nPlugin::new(&config, chat_state);
        let result = plugin.send_message("hi", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn connect_fails_without_webhook_url() {
        let config = PluginConfig {
            plugin_type: "n8n".to_string(),
            name: "Test".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: None,
            poll_url: None,
            transport: None,
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let mut plugin = N8nPlugin::new(&config, chat_state);
        let result = plugin.connect();
        assert!(result.is_err());
        assert!(result.unwrap_err().0.contains("webhook_url not configured"));
    }

    #[test]
    fn capabilities_include_chat_and_workflows() {
        let config = PluginConfig {
            plugin_type: "n8n".to_string(),
            name: "Test".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: Some("https://example.com".to_string()),
            poll_url: None,
            transport: None,
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = N8nPlugin::new(&config, chat_state);
        let caps = plugin.capabilities();
        assert!(caps.chat);
        assert!(caps.workflows);
        assert!(!caps.dashboard);
        assert!(!caps.logs);
    }

    #[test]
    fn icon_is_lightning() {
        let config = PluginConfig {
            plugin_type: "n8n".to_string(),
            name: "Test".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: Some("https://example.com".to_string()),
            poll_url: None,
            transport: None,
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        let plugin = N8nPlugin::new(&config, chat_state);
        assert_eq!(plugin.icon(), "⚡");
    }

    #[test]
    fn n8n_response_priority_response_over_output() {
        let resp = N8nResponse {
            response: Some("primary".to_string()),
            output: Some("secondary".to_string()),
            text: None,
        };
        assert_eq!(resp.text(), Some("primary"));
    }

    #[test]
    fn n8n_response_fallback_to_output() {
        let resp = N8nResponse {
            response: None,
            output: Some("fallback".to_string()),
            text: None,
        };
        assert_eq!(resp.text(), Some("fallback"));
    }

    #[test]
    fn n8n_response_fallback_to_text() {
        let resp = N8nResponse {
            response: None,
            output: None,
            text: Some("last resort".to_string()),
        };
        assert_eq!(resp.text(), Some("last resort"));
    }

    // ── BDD: chat history ───────────────────────────────────────────

    /// Helper: create an N8nPlugin with a pre-loaded history for testing.
    fn make_test_plugin() -> N8nPlugin {
        let config = PluginConfig {
            plugin_type: "n8n".to_string(),
            name: "Test".to_string(),
            url: None,
            token: None,
            model: None,
            api_key: None,
            webhook_url: Some("https://example.com/webhook".to_string()),
            poll_url: None,
            transport: None,
            command: None,
            args: None,
            system_prompt: None,
        };
        let chat_state = Arc::new(Mutex::new(ChatState::new()));
        N8nPlugin::new(&config, chat_state)
    }

    #[test]
    fn first_message_has_empty_history() {
        // Given a new n8n plugin session
        let plugin = make_test_plugin();
        // When building request for first message
        let history = plugin.history.lock().unwrap().clone();
        // Then history is empty
        assert!(history.is_empty());
    }

    #[test]
    fn second_message_includes_first_exchange() {
        // Given user sent "你好" and got reply "我是助手"
        let plugin = make_test_plugin();
        record_exchange(&plugin.history, "你好", "我是助手");

        // When user sends "列出 workflow"
        let history = plugin.history.lock().unwrap().clone();

        // Then history contains the first exchange
        assert_eq!(history.len(), 2);
        assert_eq!(
            history[0],
            HistoryEntry { role: "user".into(), content: "你好".into() }
        );
        assert_eq!(
            history[1],
            HistoryEntry { role: "assistant".into(), content: "我是助手".into() }
        );
    }

    #[test]
    fn history_capped_at_20_entries() {
        // Given 15 complete exchanges exist
        let plugin = make_test_plugin();
        for i in 0..15 {
            record_exchange(
                &plugin.history,
                &format!("user msg {i}"),
                &format!("assistant reply {i}"),
            );
        }

        // When a new message is sent
        // Then history has exactly 20 entries (last 10 turns)
        let history = plugin.history.lock().unwrap();
        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        // First entry should be from exchange #5 (0-indexed), trimmed from front
        assert_eq!(history[0].content, "user msg 5");
        assert_eq!(history[1].content, "assistant reply 5");
        // Last entry should be from exchange #14
        assert_eq!(history[18].content, "user msg 14");
        assert_eq!(history[19].content, "assistant reply 14");
    }

    #[test]
    fn history_persists_across_plugin_reference() {
        // Given user chatted with n8n plugin
        let plugin = make_test_plugin();
        record_exchange(&plugin.history, "hello", "hi there");

        // When we access history through the Arc (simulating plugin switch & back)
        let history_ref = Arc::clone(&plugin.history);
        let history = history_ref.lock().unwrap();

        // Then history is preserved
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "hello");
    }

    #[test]
    fn clear_command_resets_history() {
        // Given user has chat history
        let plugin = make_test_plugin();
        record_exchange(&plugin.history, "msg1", "reply1");
        record_exchange(&plugin.history, "msg2", "reply2");
        assert_eq!(plugin.history.lock().unwrap().len(), 4);

        // When user sends "/clear" (simulate the clear logic from send_message)
        plugin.history.lock().unwrap().clear();

        // Then local history is cleared
        assert!(plugin.history.lock().unwrap().is_empty());
    }

    #[test]
    fn clear_command_via_send_message() {
        // Given user has chat history and plugin is not connected
        let plugin = make_test_plugin();
        record_exchange(&plugin.history, "msg1", "reply1");
        assert_eq!(plugin.history.lock().unwrap().len(), 2);

        // When user sends "/clear" via send_message
        // (send_message handles /clear even without connection)
        let result = plugin.send_message("/clear", None, None);
        assert!(result.is_ok());

        // Then history is cleared
        assert!(plugin.history.lock().unwrap().is_empty());

        // And a reply was pushed to chat state
        let cs = plugin.chat_state.lock().unwrap();
        assert!(!cs.inbox.is_empty());
    }

    #[test]
    fn session_id_uses_plugin_id() {
        // The session_id in requests should be the plugin id
        let plugin = make_test_plugin();
        assert_eq!(plugin.id().0, "n8n-test");
    }
}
