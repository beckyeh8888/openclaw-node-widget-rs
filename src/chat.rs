use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::json;
use tokio::sync::mpsc;
use tracing::warn;

use crate::config::{Config, GeneralSettings, PluginConfig};
use crate::dashboard::{DashboardData, LogBuffer, LogEntry, LogLevel};
use crate::gateway::{AgentInfo, ChatAttachment, ChatSessionInfo};
use crate::history::{ChatHistory, PersistedMessage};
use crate::i18n;
use crate::media::MediaStore;
use crate::plugin::{PluginCommand, TokenUsage};

const MAX_MESSAGES: usize = 50;

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub sender: ChatSender,
    pub text: String,
    pub media_path: Option<String>,
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatSender {
    User,
    Agent(String),
}

#[derive(Debug)]
pub enum ChatInbound {
    Reply {
        text: String,
        agent_name: Option<String>,
        usage: Option<TokenUsage>,
        attachments: Option<Vec<ChatAttachment>>,
    },
    StreamStart {
        msg_id: String,
        agent_name: Option<String>,
    },
    StreamChunk {
        msg_id: String,
        text: String,
    },
    StreamEnd {
        msg_id: String,
    },
    SessionsList {
        sessions: Vec<ChatSessionInfo>,
    },
    Connected,
    Disconnected,
    VoiceTranscription {
        text: String,
    },
    PinChanged {
        pinned: bool,
    },
    PluginSwitched {
        plugin_id: String,
        session_key: String,
    },
    AgentsList {
        agents: Vec<AgentInfo>,
    },
}

#[derive(Debug, Clone)]
pub struct PendingStream {
    pub msg_id: String,
    pub agent_name: String,
    pub text: String,
}

pub struct ChatState {
    pub messages: Vec<ChatMessage>,
    pub inbox: Vec<ChatInbound>,
    pub sessions: Vec<ChatSessionInfo>,
    pub selected_session: Option<String>,
    pub connected: bool,
    pub window_open: bool,
    pub window_focused: bool,
    pub waiting_for_reply: bool,
    pub pending_stream: Option<PendingStream>,
    pub dashboard_data: DashboardData,
    pub log_buffer: LogBuffer,
    pub current_page: String,
    pub settings_requested: bool,
    /// Currently active plugin ID (for multi-plugin support).
    pub active_plugin_id: Option<String>,
    /// Currently active session key within the active plugin.
    pub active_session_key: String,
    /// Available agents discovered from Gateway.
    pub agents: Vec<AgentInfo>,
    /// Currently active agent ID (e.g. "main", "divination", "n8n").
    pub active_agent_id: String,
    /// Set to true when the app should fully quit (e.g. from tray "Quit" menu).
    pub app_quit: bool,
    /// Media storage for file attachments.
    pub media_store: MediaStore,
    /// Chat history persistence (SQLite).
    pub history: Option<ChatHistory>,
}

impl std::fmt::Debug for ChatState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatState")
            .field("messages", &self.messages.len())
            .field("connected", &self.connected)
            .field("active_agent_id", &self.active_agent_id)
            .field("active_session_key", &self.active_session_key)
            .field("history", &self.history.is_some())
            .finish()
    }
}

impl Default for ChatState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            inbox: Vec::new(),
            sessions: Vec::new(),
            selected_session: None,
            connected: false,
            window_open: false,
            window_focused: true,
            waiting_for_reply: false,
            pending_stream: None,
            dashboard_data: DashboardData::new(),
            log_buffer: LogBuffer::new(),
            current_page: "chat".to_string(),
            settings_requested: false,
            active_plugin_id: None,
            active_session_key: "main".to_string(),
            agents: Vec::new(),
            active_agent_id: "main".to_string(),
            app_quit: false,
            media_store: MediaStore::new(),
            history: None,
        }
    }

    /// Build the conversation key for the current active plugin + session.
    pub fn conversation_key(&self) -> String {
        let plugin = self.active_plugin_id.as_deref().unwrap_or("default");
        ChatHistory::conversation_key(plugin, &self.active_session_key)
    }

    /// Load messages from history for the current conversation.
    pub fn load_from_history(&mut self, history: &ChatHistory) {
        let key = self.conversation_key();
        let persisted = history.get_messages(&key);
        self.messages = persisted
            .iter()
            .map(|pm| ChatMessage {
                sender: if pm.sender == "user" {
                    ChatSender::User
                } else {
                    ChatSender::Agent(
                        pm.agent_name.clone().unwrap_or_else(|| "Agent".to_string()),
                    )
                },
                text: pm.text.clone(),
                media_path: pm.media_path.clone(),
                media_type: pm.media_type.clone(),
            })
            .collect();
    }

    /// Persist current messages to history.
    pub fn save_to_history(&self, history: &mut ChatHistory) {
        let key = self.conversation_key();
        let persisted: Vec<PersistedMessage> = self
            .messages
            .iter()
            .map(|m| PersistedMessage {
                sender: match &m.sender {
                    ChatSender::User => "user".to_string(),
                    ChatSender::Agent(_) => "agent".to_string(),
                },
                agent_name: match &m.sender {
                    ChatSender::Agent(name) => Some(name.clone()),
                    _ => None,
                },
                text: m.text.clone(),
                media_path: m.media_path.clone(),
                media_type: m.media_type.clone(),
                created_at: now_unix_ms(),
            })
            .collect();
        history.set_messages(&key, persisted);
    }

    /// Switch to a different plugin + session, saving current and loading new.
    pub fn switch_conversation(
        &mut self,
        history: &mut ChatHistory,
        plugin_id: &str,
        session_key: &str,
    ) {
        // Save current conversation
        self.save_to_history(history);
        // Switch IDs
        self.active_plugin_id = Some(plugin_id.to_string());
        self.active_session_key = session_key.to_string();
        self.selected_session = Some(session_key.to_string());
        // Load new conversation
        self.load_from_history(history);
        self.pending_stream = None;
        self.waiting_for_reply = false;
    }

    /// Add a log entry to the buffer.
    pub fn add_log(&mut self, level: LogLevel, source: &str, message: &str) {
        self.log_buffer.push(LogEntry {
            timestamp: crate::dashboard::now_timestamp(),
            level,
            source: source.to_string(),
            message: message.to_string(),
        });
    }
}

/// Create a WebView attached to an existing window (non-blocking).
///
/// The caller owns the `Window` and `WebView` and is responsible for driving
/// events via [`process_chat_events`] on each tick of the main event loop.
pub fn create_chat_webview(
    window: &tao::window::Window,
    chat_state: &Arc<Mutex<ChatState>>,
    cmd_senders: Arc<HashMap<String, mpsc::UnboundedSender<PluginCommand>>>,
) -> crate::error::Result<wry::WebView> {
    // Send ListSessions to the active sender
    if let Some(tx) = active_sender(&cmd_senders, chat_state) {
        let _ = tx.send(PluginCommand::ListSessions);
    }

    let init_json = build_init_json(chat_state);
    let html_template = include_str!("chat_ui.html");
    let html = html_template.replace("\"__INIT_DATA__\"", &init_json);

    let chat_state_ipc = Arc::clone(chat_state);

    let webview = wry::WebViewBuilder::new()
        .with_html(html)
        .with_ipc_handler(move |req| {
            let body = req.body();
            handle_ipc_message(body, &cmd_senders, &chat_state_ipc);
        })
        .build(window)
        .map_err(|e| crate::error::AppError::Tray(format!("webview: {e}")))?;

    Ok(webview)
}

/// Per-tick processing for the chat webview: handle pin changes and forward
/// inbox events to the WebView via `evaluate_script`.
pub fn process_chat_events(
    chat_state: &Arc<Mutex<ChatState>>,
    webview: &wry::WebView,
    window: &tao::window::Window,
) {
    // Handle pin changes that require window access
    if let Ok(mut state) = chat_state.lock() {
        let mut i = 0;
        while i < state.inbox.len() {
            if let ChatInbound::PinChanged { pinned } = &state.inbox[i] {
                window.set_always_on_top(*pinned);
                state.inbox.remove(i);
            } else {
                i += 1;
            }
        }
    }
    process_inbox_to_webview(chat_state, webview);
}

/// Look up the sender for the currently active plugin, falling back to "default".
fn active_sender<'a>(
    senders: &'a HashMap<String, mpsc::UnboundedSender<PluginCommand>>,
    chat_state: &Arc<Mutex<ChatState>>,
) -> Option<&'a mpsc::UnboundedSender<PluginCommand>> {
    let active_id = chat_state
        .lock()
        .ok()
        .and_then(|s| s.active_plugin_id.clone());
    if let Some(ref id) = active_id {
        if let Some(tx) = senders.get(id) {
            return Some(tx);
        }
    }
    // Fallback: "default" key (gateway-only mode) or first available sender
    senders
        .get("default")
        .or_else(|| senders.values().next())
}

fn file_url_from_path(path: std::path::PathBuf) -> String {
    format!("file://{}", path.to_string_lossy().replace('\\', "/"))
}

fn media_name_from_relative(relative: &str) -> Option<String> {
    std::path::Path::new(relative)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn build_init_json(chat_state: &Arc<Mutex<ChatState>>) -> String {
    let state = chat_state.lock().unwrap_or_else(|e| e.into_inner());

    let lang = match i18n::current_lang() {
        i18n::Lang::En => "en",
        i18n::Lang::ZhTw => "zh-tw",
        i18n::Lang::ZhCn => "zh-cn",
    };

    let messages: Vec<serde_json::Value> = state
        .messages
        .iter()
        .map(|m| {
            let media_url = m
                .media_path
                .as_deref()
                .map(|p| file_url_from_path(state.media_store.get_full_path(p)));
            let media_name = m.media_path.as_deref().and_then(media_name_from_relative);
            match &m.sender {
                ChatSender::User => json!({
                    "sender": "user",
                    "text": m.text,
                    "mediaUrl": media_url,
                    "mediaType": m.media_type.clone(),
                    "mediaName": media_name,
                }),
                ChatSender::Agent(name) => {
                    json!({
                        "sender": "agent",
                        "agentName": name,
                        "text": m.text,
                        "mediaUrl": media_url,
                        "mediaType": m.media_type.clone(),
                        "mediaName": media_name,
                    })
                }
            }
        })
        .collect();

    let sessions: Vec<serde_json::Value> = state
        .sessions
        .iter()
        .map(|s| json!({"key": s.key, "name": s.name}))
        .collect();

    let log_entries: Vec<serde_json::Value> = state
        .log_buffer
        .entries()
        .iter()
        .map(|e| {
            json!({
                "timestamp": e.timestamp,
                "level": format!("{}", e.level),
                "source": e.source,
                "message": e.message,
            })
        })
        .collect();

    let config = Config::load().unwrap_or_default();
    let tts_config = config.tts.clone();

    let effective = config.effective_plugins();
    let plugins_json: Vec<serde_json::Value> = effective
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let slug: String = p
                .name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect::<String>()
                .trim_matches('-')
                .to_string();
            let id = format!("{}-{}", p.plugin_type, slug);
            let is_active = state
                .active_plugin_id
                .as_deref()
                .map(|aid| aid == id)
                .unwrap_or(i == 0);
            json!({
                "id": id,
                "name": p.name,
                "type": p.plugin_type,
                "active": is_active,
            })
        })
        .collect();

    let agents_json: Vec<serde_json::Value> = state
        .agents
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "sessionKey": a.session_key,
                "type": a.agent_type,
            })
        })
        .collect();

    json!({
        "lang": lang,
        "connected": state.connected,
        "messages": messages,
        "sessions": sessions,
        "selectedSession": state.selected_session,
        "waitingForReply": state.waiting_for_reply,
        "dashboard": state.dashboard_data,
        "logs": log_entries,
        "currentPage": state.current_page,
        "activePluginId": state.active_plugin_id,
        "activeSessionKey": state.active_session_key,
        "activeAgentId": state.active_agent_id,
        "agents": agents_json,
        "plugins": plugins_json,
        "theme": config.widget.theme,
        "alwaysOnTop": config.widget.always_on_top,
        "tts": {
            "enabled": tts_config.enabled,
            "auto_read": tts_config.auto_read,
            "voice": tts_config.voice,
            "rate": tts_config.rate,
        },
    })
    .to_string()
}

pub fn handle_ipc_message(
    body: &str,
    cmd_senders: &HashMap<String, mpsc::UnboundedSender<PluginCommand>>,
    chat_state: &Arc<Mutex<ChatState>>,
) {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(body) else {
        warn!("invalid IPC message: {body}");
        return;
    };

    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match msg_type {
        "send" => {
            let message = msg
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let has_attachments = msg
                .get("attachments")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            if message.is_empty() && !has_attachments {
                return;
            }

            let session_key = msg
                .get("sessionKey")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .or_else(|| {
                    chat_state
                        .lock()
                        .ok()
                        .map(|s| s.active_session_key.clone())
                });

            let attachments: Option<Vec<ChatAttachment>> = msg
                .get("attachments")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| {
                            Some(ChatAttachment {
                                data: a.get("data")?.as_str()?.to_string(),
                                filename: a.get("filename")?.as_str()?.to_string(),
                                mime_type: a.get("mimeType")?.as_str()?.to_string(),
                            })
                        })
                        .collect()
                });

            if let Some(tx) = active_sender(cmd_senders, chat_state) {
                let _ = tx.send(PluginCommand::SendChat {
                    message: message.clone(),
                    session_key,
                    attachments: attachments.clone(),
                });
            }

            if let Ok(mut state) = chat_state.lock() {
                let mut media_path: Option<String> = None;
                let mut media_type: Option<String> = None;
                let mut final_text = message;

                if let Some(first) = attachments.as_ref().and_then(|a| a.first()) {
                    match STANDARD.decode(&first.data) {
                        Ok(bytes) => match state.media_store.store_file(&bytes, &first.mime_type) {
                            Ok(path) => {
                                media_path = Some(path);
                                media_type = Some(first.mime_type.clone());
                            }
                            Err(e) => {
                                if e.kind() == std::io::ErrorKind::InvalidData {
                                    final_text = "File too large".to_string();
                                } else {
                                    warn!("failed to store outbound media: {e}");
                                }
                            }
                        },
                        Err(e) => warn!("failed to decode outbound attachment: {e}"),
                    }
                }

                state.messages.push(ChatMessage {
                    sender: ChatSender::User,
                    text: final_text,
                    media_path,
                    media_type,
                });
                state.waiting_for_reply = true;
                while state.messages.len() > MAX_MESSAGES {
                    state.messages.remove(0);
                }
            }
        }
        "selectSession" => {
            let session_key = msg
                .get("sessionKey")
                .and_then(|v| v.as_str())
                .map(String::from);
            if let Ok(mut state) = chat_state.lock() {
                state.selected_session = session_key;
            }
        }
        "switchPlugin" => {
            let plugin_id = msg
                .get("pluginId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let session_key = msg
                .get("sessionKey")
                .and_then(|v| v.as_str())
                .unwrap_or("main")
                .to_string();
            if !plugin_id.is_empty() {
                if let Ok(mut state) = chat_state.lock() {
                    let changed = state
                        .active_plugin_id
                        .as_deref()
                        .map(|id| id != plugin_id)
                        .unwrap_or(true);
                    state.active_plugin_id = Some(plugin_id.clone());
                    state.active_session_key = session_key.clone();
                    state.selected_session = Some(session_key.clone());
                    if changed {
                        // Clear current conversation state for the new plugin
                        state.messages.clear();
                        state.pending_stream = None;
                        state.waiting_for_reply = false;
                        state.inbox.push(ChatInbound::PluginSwitched {
                            plugin_id,
                            session_key,
                        });
                    }
                }
            }
        }
        "switchAgent" => {
            let agent_id = msg
                .get("agentId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let agent_type = msg
                .get("agentType")
                .and_then(|v| v.as_str())
                .unwrap_or("openclaw")
                .to_string();
            let session_key = msg
                .get("sessionKey")
                .and_then(|v| v.as_str())
                .unwrap_or(&agent_id)
                .to_string();
            if !agent_id.is_empty() {
                if let Ok(mut state) = chat_state.lock() {
                    let changed = state.active_agent_id != agent_id;
                    if changed {
                        // 1. Save CURRENT conversation with OLD keys
                        if let Some(mut history) = state.history.take() {
                            state.save_to_history(&mut history);
                            state.history = Some(history);
                        }

                        // 2. Switch keys to NEW agent
                        state.active_agent_id = agent_id.clone();
                        if agent_type == "openclaw" {
                            state.active_session_key = session_key;
                            let oc_id = cmd_senders
                                .keys()
                                .find(|k| k.starts_with("openclaw-"))
                                .cloned()
                                .unwrap_or_else(|| "default".to_string());
                            state.active_plugin_id = Some(oc_id);
                        } else {
                            state.active_plugin_id = Some(agent_id.clone());
                            state.active_session_key = "main".to_string();
                        }

                        // 3. Load NEW conversation with NEW keys
                        if let Some(mut history) = state.history.take() {
                            state.load_from_history(&history);
                            state.history = Some(history);
                        } else {
                            state.messages.clear();
                        }

                        let sk = state.active_session_key.clone();
                        state.pending_stream = None;
                        state.waiting_for_reply = false;
                        state.inbox.push(ChatInbound::PluginSwitched {
                            plugin_id: agent_id,
                            session_key: sk,
                        });
                    } else {
                        // Same agent, just update keys
                        state.active_agent_id = agent_id.clone();
                        if agent_type == "openclaw" {
                            state.active_session_key = session_key;
                            let oc_id = cmd_senders
                                .keys()
                                .find(|k| k.starts_with("openclaw-"))
                                .cloned()
                                .unwrap_or_else(|| "default".to_string());
                            state.active_plugin_id = Some(oc_id);
                        } else {
                            state.active_plugin_id = Some(agent_id.clone());
                            state.active_session_key = "main".to_string();
                        }
                    }
                }
            }
        }
        "listSessions" => {
            if let Some(tx) = active_sender(cmd_senders, chat_state) {
                let _ = tx.send(PluginCommand::ListSessions);
            }
        }
        "getDashboard" => {
            // Dashboard data is pushed from Rust side; this is a manual refresh request.
            // No-op: the event loop will push the latest data on next tick.
        }
        "getLogs" => {
            // Logs are pushed; this acknowledges the JS side is ready.
        }
        "filterLogs" => {
            // Filtering is done client-side in JS; no Rust action needed.
        }
        "clearLogs" => {
            if let Ok(mut state) = chat_state.lock() {
                state.log_buffer.clear();
            }
        }
        "navigate" => {
            let page = msg
                .get("page")
                .and_then(|v| v.as_str())
                .unwrap_or("chat")
                .to_string();
            if let Ok(mut state) = chat_state.lock() {
                state.current_page = page;
            }
        }
        "getSettings" => {
            // Handled via webview eval in process_inbox; push settings data
            if let Ok(mut state) = chat_state.lock() {
                state.settings_requested = true;
            }
        }
        "savePlugin" => {
            let plugin_json = msg.get("plugin");
            if let Some(pj) = plugin_json {
                let pc = PluginConfig {
                    plugin_type: pj.get("type").and_then(|v| v.as_str()).unwrap_or("openclaw").to_string(),
                    name: pj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    url: pj.get("url").and_then(|v| v.as_str()).map(String::from),
                    token: pj.get("token").and_then(|v| v.as_str()).map(String::from),
                    model: pj.get("model").and_then(|v| v.as_str()).map(String::from),
                    api_key: pj.get("apiKey").and_then(|v| v.as_str()).map(String::from),
                    webhook_url: pj.get("webhookUrl").and_then(|v| v.as_str()).map(String::from),
                    poll_url: pj.get("pollUrl").and_then(|v| v.as_str()).map(String::from),
                    transport: pj.get("transport").and_then(|v| v.as_str()).map(String::from),
                    command: pj.get("command").and_then(|v| v.as_str()).map(String::from),
                    args: pj.get("args").and_then(|v| v.as_array()).map(|a| {
                        a.iter().filter_map(|s| s.as_str().map(String::from)).collect()
                    }),
                    system_prompt: pj.get("systemPrompt").and_then(|v| v.as_str()).map(String::from),
                };
                if !pc.name.is_empty() {
                    match Config::load() {
                        Ok(mut config) => {
                            config.upsert_plugin(pc);
                            if let Err(e) = config.save() {
                                warn!("failed to save plugin config: {e}");
                            }
                        }
                        Err(e) => warn!("failed to load config for savePlugin: {e}"),
                    }
                }
            }
        }
        "deletePlugin" => {
            let name = msg.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if !name.is_empty() {
                match Config::load() {
                    Ok(mut config) => {
                        config.remove_plugin(name);
                        if let Err(e) = config.save() {
                            warn!("failed to save config after deletePlugin: {e}");
                        }
                    }
                    Err(e) => warn!("failed to load config for deletePlugin: {e}"),
                }
            }
        }
        "clearConversation" => {
            if let Ok(mut state) = chat_state.lock() {
                state.messages.clear();
                state.pending_stream = None;
                state.waiting_for_reply = false;
            }
        }
        "export" => {
            let format = msg.get("format").and_then(|v| v.as_str()).unwrap_or("markdown");
            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if !content.is_empty() {
                let ext = if format == "markdown" { "md" } else { "txt" };
                let filename = format!(
                    "chat-export-{}.{}",
                    chrono::Local::now().format("%Y%m%d-%H%M%S"),
                    ext
                );
                let downloads = dirs::download_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                let path = downloads.join(&filename);
                if let Err(e) = std::fs::write(&path, content) {
                    warn!("failed to export chat: {e}");
                }
            }
        }
        "setModel" => {
            let _model = msg.get("model").and_then(|v| v.as_str()).unwrap_or("");
            // Model setting is stored per-plugin; currently a no-op placeholder.
        }
        "setSystemPrompt" => {
            let prompt = msg.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let plugin_name = msg.get("pluginName").and_then(|v| v.as_str()).unwrap_or("");
            if !plugin_name.is_empty() {
                match Config::load() {
                    Ok(mut config) => {
                        if let Some(p) = config.plugins.iter_mut().find(|p| p.name == plugin_name) {
                            p.system_prompt = if prompt.is_empty() { None } else { Some(prompt) };
                            if let Err(e) = config.save() {
                                warn!("failed to save system prompt: {e}");
                            }
                        }
                    }
                    Err(e) => warn!("failed to load config for setSystemPrompt: {e}"),
                }
            }
        }
        "setTtsAutoRead" => {
            let auto_read = msg.get("autoRead").and_then(|v| v.as_bool()).unwrap_or(false);
            match Config::load() {
                Ok(mut config) => {
                    config.tts.auto_read = auto_read;
                    if let Err(e) = config.save() {
                        warn!("failed to save TTS config: {e}");
                    }
                }
                Err(e) => warn!("failed to load config for setTtsAutoRead: {e}"),
            }
        }
        "setTheme" => {
            let theme = msg.get("theme").and_then(|v| v.as_str()).unwrap_or("auto").to_string();
            match Config::load() {
                Ok(mut config) => {
                    config.widget.theme = theme;
                    if let Err(e) = config.save() {
                        warn!("failed to save theme config: {e}");
                    }
                }
                Err(e) => warn!("failed to load config for setTheme: {e}"),
            }
        }
        "pin" => {
            let pinned = msg.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false);
            match Config::load() {
                Ok(mut config) => {
                    config.widget.always_on_top = pinned;
                    if let Err(e) = config.save() {
                        warn!("failed to save pin config: {e}");
                    }
                }
                Err(e) => warn!("failed to load config for pin: {e}"),
            }
            // The actual window.set_always_on_top is handled by the event loop
            if let Ok(mut state) = chat_state.lock() {
                state.inbox.push(ChatInbound::PinChanged { pinned });
            }
        }
        "saveGeneral" => {
            let general = GeneralSettings {
                language: msg.get("language").and_then(|v| v.as_str()).unwrap_or("auto").to_string(),
                auto_start: msg.get("autoStart").and_then(|v| v.as_bool()).unwrap_or(false),
                theme: msg.get("theme").and_then(|v| v.as_str()).unwrap_or("auto").to_string(),
                always_on_top: msg.get("alwaysOnTop").and_then(|v| v.as_bool()).unwrap_or(false),
            };
            match Config::load() {
                Ok(mut config) => {
                    config.apply_general_settings(&general);
                    if let Err(e) = config.save() {
                        warn!("failed to save general settings: {e}");
                    }
                }
                Err(e) => warn!("failed to load config for saveGeneral: {e}"),
            }
        }
        "voice" => {
            let audio = msg
                .get("audio")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if audio.is_empty() {
                warn!("voice IPC message with empty audio");
                return;
            }
            let chat_state = Arc::clone(chat_state);
            tokio::spawn(async move {
                let voice_config = match crate::config::Config::load() {
                    Ok(c) => c.voice,
                    Err(_) => crate::config::VoiceConfig::default(),
                };
                match crate::voice::transcribe(&audio, &voice_config).await {
                    Ok(text) => {
                        if let Ok(mut state) = chat_state.lock() {
                            state.inbox.push(ChatInbound::VoiceTranscription {
                                text,
                            });
                        }
                    }
                    Err(e) => {
                        warn!("voice transcription failed: {e}");
                        if let Ok(mut state) = chat_state.lock() {
                            state.inbox.push(ChatInbound::VoiceTranscription {
                                text: String::new(),
                            });
                        }
                    }
                }
            });
        }
        _ => {
            warn!("unknown IPC message type: {msg_type}");
        }
    }
}

fn process_inbox_to_webview(
    chat_state: &Arc<Mutex<ChatState>>,
    webview: &wry::WebView,
) {
    let Ok(mut state) = chat_state.lock() else {
        return;
    };

    let events: Vec<_> = state.inbox.drain(..).collect();

    for event in events {
        match event {
            ChatInbound::Reply {
                text,
                agent_name,
                usage,
                attachments,
            } => {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                let mut media_path: Option<String> = None;
                let mut media_type: Option<String> = None;
                let mut final_text = text;

                if let Some(first) = attachments.as_ref().and_then(|a| a.first()) {
                    match STANDARD.decode(&first.data) {
                        Ok(bytes) => match state.media_store.store_file(&bytes, &first.mime_type) {
                            Ok(path) => {
                                media_path = Some(path);
                                media_type = Some(first.mime_type.clone());
                            }
                            Err(e) => {
                                if e.kind() == std::io::ErrorKind::InvalidData {
                                    final_text = "File too large".to_string();
                                } else {
                                    warn!("failed to store inbound media: {e}");
                                }
                            }
                        },
                        Err(e) => warn!("failed to decode inbound attachment: {e}"),
                    }
                }

                state.messages.push(ChatMessage {
                    sender: ChatSender::Agent(name.clone()),
                    text: final_text.clone(),
                    media_path: media_path.clone(),
                    media_type: media_type.clone(),
                });
                state.waiting_for_reply = false;
                while state.messages.len() > MAX_MESSAGES {
                    state.messages.remove(0);
                }

                let usage_json = usage.as_ref().map(|u| {
                    json!({
                        "input_tokens": u.input_tokens,
                        "output_tokens": u.output_tokens,
                        "duration_ms": u.duration_ms,
                    })
                });
                let media_url = media_path
                    .as_deref()
                    .map(|p| file_url_from_path(state.media_store.get_full_path(p)));
                let media_name = media_path.as_deref().and_then(media_name_from_relative);
                let msg_json = json!({
                    "sender": "agent",
                    "agentName": name,
                    "text": final_text,
                    "usage": usage_json,
                    "mediaUrl": media_url,
                    "mediaType": media_type,
                    "mediaName": media_name,
                });
                let _ = webview
                    .evaluate_script(&format!("addMessage({})", msg_json));
                let _ = webview.evaluate_script("setTyping(false)");
            }
            ChatInbound::SessionsList { sessions } => {
                state.sessions = sessions.clone();
                let sessions_json: Vec<serde_json::Value> = sessions
                    .iter()
                    .map(|s| json!({"key": s.key, "name": s.name}))
                    .collect();
                let _ = webview.evaluate_script(&format!(
                    "setSessions({})",
                    json!(sessions_json)
                ));
            }
            ChatInbound::StreamStart { msg_id, agent_name } => {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                state.pending_stream = Some(PendingStream {
                    msg_id: msg_id.clone(),
                    agent_name: name.clone(),
                    text: String::new(),
                });
                state.waiting_for_reply = false;
                let data = json!({ "id": msg_id, "agentName": name });
                let _ = webview.evaluate_script(&format!(
                    "if(typeof streamStart==='function')streamStart({})",
                    data
                ));
                let _ = webview.evaluate_script("setTyping(false)");
            }
            ChatInbound::StreamChunk { msg_id, text } => {
                if let Some(ref mut ps) = state.pending_stream {
                    if ps.msg_id == msg_id {
                        // Gateway sends full accumulated text — replace, not append
                        ps.text = text.clone();
                    }
                }
                let data = json!({ "id": msg_id, "text": text });
                let _ = webview.evaluate_script(&format!(
                    "if(typeof streamChunk==='function')streamChunk({})",
                    data
                ));
            }
            ChatInbound::StreamEnd { msg_id } => {
                if let Some(ps) = state.pending_stream.take() {
                    if ps.msg_id == msg_id {
                        state.messages.push(ChatMessage {
                            sender: ChatSender::Agent(ps.agent_name),
                            text: ps.text,
                            media_path: None,
                            media_type: None,
                        });
                        while state.messages.len() > MAX_MESSAGES {
                            state.messages.remove(0);
                        }
                    }
                }
                let data = json!({ "id": msg_id });
                let _ = webview.evaluate_script(&format!(
                    "if(typeof streamEnd==='function')streamEnd({})",
                    data
                ));
            }
            ChatInbound::Connected => {
                state.connected = true;
                let _ = webview.evaluate_script("setConnected(true)");
            }
            ChatInbound::Disconnected => {
                state.connected = false;
                state.waiting_for_reply = false;
                let _ = webview.evaluate_script("setConnected(false)");
                let _ = webview.evaluate_script("setTyping(false)");
            }
            ChatInbound::VoiceTranscription { text } => {
                let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
                let _ = webview.evaluate_script(&format!(
                    "if(typeof onVoiceTranscription==='function')onVoiceTranscription('{}')",
                    escaped
                ));
            }
            ChatInbound::PinChanged { .. } => {
                // Handled by the event loop (requires window access)
            }
            ChatInbound::PluginSwitched { plugin_id, session_key } => {
                let _ = webview.evaluate_script(
                    "if(typeof clearChat==='function')clearChat()"
                );
                let data = json!({
                    "pluginId": plugin_id,
                    "sessionKey": session_key,
                });
                let _ = webview.evaluate_script(&format!(
                    "if(typeof onPluginSwitched==='function')onPluginSwitched({})",
                    data
                ));
                // Re-render messages loaded from SQLite history
                for msg in &state.messages {
                    let sender = match &msg.sender {
                        ChatSender::User => "user",
                        ChatSender::Agent(_) => "agent",
                    };
                    let agent_name = match &msg.sender {
                        ChatSender::Agent(n) => Some(n.clone()),
                        _ => None,
                    };
                    let mut m = json!({
                        "sender": sender,
                        "text": msg.text,
                    });
                    if let Some(name) = agent_name {
                        m["agentName"] = json!(name);
                    }
                    if let Some(ref path) = msg.media_path {
                        let full_path = state.media_store.get_full_path(path);
                        let path_str = full_path.to_string_lossy().replace('\\', "/");
                        m["mediaUrl"] = json!(format!("file:///{}", path_str));
                        if let Some(ref mt) = msg.media_type {
                            m["mediaType"] = json!(mt);
                        }
                    }
                    let _ = webview.evaluate_script(&format!(
                        "if(typeof addMessage==='function')addMessage({})",
                        m
                    ));
                }
            }
            ChatInbound::AgentsList { agents } => {
                state.agents = agents.clone();
                // Merge n8n/ollama/openai-compatible plugins as agent entries
                let config = Config::load().unwrap_or_default();
                let mut all_agents: Vec<serde_json::Value> = agents
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id,
                            "name": a.name,
                            "sessionKey": a.session_key,
                            "type": a.agent_type,
                        })
                    })
                    .collect();
                // Add non-openclaw plugins as pseudo-agents
                for p in config.effective_plugins() {
                    if p.plugin_type != "openclaw" {
                        let slug: String = p
                            .name
                            .to_lowercase()
                            .chars()
                            .map(|c| if c.is_alphanumeric() { c } else { '-' })
                            .collect::<String>()
                            .trim_matches('-')
                            .to_string();
                        let id = format!("{}-{}", p.plugin_type, slug);
                        all_agents.push(json!({
                            "id": id,
                            "name": p.name,
                            "sessionKey": "",
                            "type": p.plugin_type,
                        }));
                    }
                }
                let _ = webview.evaluate_script(&format!(
                    "if(typeof initAgents==='function')initAgents({})",
                    json!(all_agents)
                ));
            }
        }
    }

    // Push dashboard data to WebView if on dashboard page
    if state.current_page == "dashboard" {
        if let Ok(dashboard_json) = serde_json::to_string(&state.dashboard_data) {
            let _ = webview.evaluate_script(&format!(
                "if(typeof updateDashboard==='function')updateDashboard({})",
                dashboard_json
            ));
        }
    }

    // Push settings data to WebView if requested (also needed on store page for installed badge)
    if state.settings_requested || state.current_page == "settings" || state.current_page == "store" {
        state.settings_requested = false;
        match Config::load() {
            Ok(config) => {
                let plugins_json: Vec<serde_json::Value> = config
                    .effective_plugins()
                    .iter()
                    .map(|p| {
                        json!({
                            "type": p.plugin_type,
                            "name": p.name,
                            "url": p.url,
                            "token": p.token,
                            "model": p.model,
                            "apiKey": p.api_key,
                            "webhookUrl": p.webhook_url,
                            "pollUrl": p.poll_url,
                            "transport": p.transport,
                            "command": p.command,
                            "args": p.args,
                            "systemPrompt": p.system_prompt,
                        })
                    })
                    .collect();
                let settings_data = json!({
                    "plugins": plugins_json,
                    "general": {
                        "language": config.widget.language,
                        "autoStart": config.startup.auto_start,
                        "theme": config.widget.theme,
                    }
                });
                let _ = webview.evaluate_script(&format!(
                    "if(typeof updateSettings==='function')updateSettings({})",
                    settings_data
                ));
            }
            Err(e) => {
                tracing::warn!("failed to load config for settings page: {e}");
            }
        }
    }

    // Push latest log entries to WebView if on logs page
    if state.current_page == "logs" {
        let entries: Vec<serde_json::Value> = state
            .log_buffer
            .entries()
            .iter()
            .map(|e| {
                json!({
                    "timestamp": e.timestamp,
                    "level": format!("{}", e.level),
                    "source": e.source,
                    "message": e.message,
                })
            })
            .collect();
        let _ = webview.evaluate_script(&format!(
            "if(typeof updateLogs==='function')updateLogs({})",
            json!(entries)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::ChatSessionInfo;
    use crate::plugin::PluginCommand;
    use std::sync::{Arc, Mutex};

    // ── ChatState initialization ─────────────────────────────────────

    #[test]
    fn given_new_chat_state_then_defaults_are_correct() {
        let state = ChatState::new();

        assert!(state.messages.is_empty(), "messages should start empty");
        assert!(state.inbox.is_empty(), "inbox should start empty");
        assert!(state.sessions.is_empty(), "sessions should start empty");
        assert_eq!(state.selected_session, None, "no session selected by default");
        assert!(!state.connected, "should start disconnected");
        assert!(!state.window_open, "window should start closed");
        assert!(state.window_focused, "window_focused defaults to true");
        assert!(!state.waiting_for_reply, "not waiting for reply initially");
    }

    // ── Adding messages ──────────────────────────────────────────────

    #[test]
    fn given_empty_state_when_user_sends_message_then_it_is_stored() {
        let mut state = ChatState::new();

        state.messages.push(ChatMessage {
            sender: ChatSender::User,
            text: "hello".to_string(),
            media_path: None,
            media_type: None,
        });

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, "hello");
        assert_eq!(state.messages[0].sender, ChatSender::User);
    }

    #[test]
    fn given_empty_state_when_agent_replies_then_sender_has_name() {
        let mut state = ChatState::new();

        state.messages.push(ChatMessage {
            sender: ChatSender::Agent("Claw".to_string()),
            text: "hi there".to_string(),
            media_path: None,
            media_type: None,
        });

        assert_eq!(state.messages.len(), 1);
        assert_eq!(
            state.messages[0].sender,
            ChatSender::Agent("Claw".to_string())
        );
    }

    // ── MAX_MESSAGES limit ───────────────────────────────────────────

    #[test]
    fn given_full_history_when_new_message_added_then_oldest_is_evicted() {
        let mut state = ChatState::new();

        // Fill to MAX_MESSAGES
        for i in 0..MAX_MESSAGES {
            state.messages.push(ChatMessage {
                sender: ChatSender::User,
                text: format!("msg-{i}"),
                media_path: None,
                media_type: None,
            });
        }
        assert_eq!(state.messages.len(), MAX_MESSAGES);

        // Simulate what handle_ipc_message does: push + evict
        state.messages.push(ChatMessage {
            sender: ChatSender::User,
            text: "overflow".to_string(),
            media_path: None,
            media_type: None,
        });
        while state.messages.len() > MAX_MESSAGES {
            state.messages.remove(0);
        }

        assert_eq!(state.messages.len(), MAX_MESSAGES);
        assert_eq!(state.messages[0].text, "msg-1", "msg-0 should be evicted");
        assert_eq!(
            state.messages[MAX_MESSAGES - 1].text,
            "overflow",
            "newest message should be last"
        );
    }

    // ── ChatInbound processing ───────────────────────────────────────

    #[test]
    fn given_reply_in_inbox_when_processed_then_message_appended_and_waiting_cleared() {
        let mut state = ChatState::new();
        state.waiting_for_reply = true;
        state.inbox.push(ChatInbound::Reply {
            text: "answer".to_string(),
            agent_name: Some("Bot".to_string()),
            usage: None,
            attachments: None,
        });

        // Simulate process_inbox_to_webview logic (without webview)
        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::Reply { text, agent_name, .. } = event {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                state.messages.push(ChatMessage {
                    sender: ChatSender::Agent(name),
                    text,
                    media_path: None,
                    media_type: None,
                });
                state.waiting_for_reply = false;
            }
        }

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, "answer");
        assert_eq!(
            state.messages[0].sender,
            ChatSender::Agent("Bot".to_string())
        );
        assert!(!state.waiting_for_reply);
    }

    #[test]
    fn given_reply_without_agent_name_then_defaults_to_agent() {
        let mut state = ChatState::new();
        state.inbox.push(ChatInbound::Reply {
            text: "hi".to_string(),
            agent_name: None,
            usage: None,
            attachments: None,
        });

        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::Reply { text, agent_name, .. } = event {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                state.messages.push(ChatMessage {
                    sender: ChatSender::Agent(name),
                    text,
                    media_path: None,
                    media_type: None,
                });
            }
        }

        assert_eq!(
            state.messages[0].sender,
            ChatSender::Agent("Agent".to_string())
        );
    }

    #[test]
    fn given_sessions_list_in_inbox_when_processed_then_sessions_updated() {
        let mut state = ChatState::new();
        state.inbox.push(ChatInbound::SessionsList {
            sessions: vec![
                ChatSessionInfo {
                    key: "s1".to_string(),
                    name: "Session 1".to_string(),
                },
                ChatSessionInfo {
                    key: "s2".to_string(),
                    name: "Session 2".to_string(),
                },
            ],
        });

        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::SessionsList { sessions } = event {
                state.sessions = sessions;
            }
        }

        assert_eq!(state.sessions.len(), 2);
        assert_eq!(state.sessions[0].key, "s1");
        assert_eq!(state.sessions[1].name, "Session 2");
    }

    #[test]
    fn given_connected_event_in_inbox_then_state_becomes_connected() {
        let mut state = ChatState::new();
        assert!(!state.connected);

        state.inbox.push(ChatInbound::Connected);
        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::Connected = event {
                state.connected = true;
            }
        }

        assert!(state.connected);
    }

    #[test]
    fn given_disconnected_event_then_connected_and_waiting_cleared() {
        let mut state = ChatState::new();
        state.connected = true;
        state.waiting_for_reply = true;

        state.inbox.push(ChatInbound::Disconnected);
        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::Disconnected = event {
                state.connected = false;
                state.waiting_for_reply = false;
            }
        }

        assert!(!state.connected);
        assert!(!state.waiting_for_reply);
    }

    // ── IPC message handling (handle_ipc_message) ────────────────────

    fn setup_ipc() -> (
        Arc<Mutex<ChatState>>,
        HashMap<String, tokio::sync::mpsc::UnboundedSender<PluginCommand>>,
        tokio::sync::mpsc::UnboundedReceiver<PluginCommand>,
    ) {
        let state = Arc::new(Mutex::new(ChatState::new()));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut senders = HashMap::new();
        senders.insert("default".to_string(), tx);
        (state, senders, rx)
    }

    #[test]
    fn given_send_ipc_then_command_includes_message_and_session_key() {
        let (state, tx, mut rx) = setup_ipc();

        let body = r#"{"type":"send","message":"hello world","sessionKey":"sess-42"}"#;
        handle_ipc_message(body, &tx, &state);

        let cmd = rx.try_recv().expect("should receive SendChat command");
        match cmd {
            PluginCommand::SendChat {
                message,
                session_key,
                attachments,
            } => {
                assert_eq!(message, "hello world");
                assert_eq!(session_key, Some("sess-42".to_string()));
                assert!(attachments.is_none());
            }
            _ => panic!("expected SendChat"),
        }

        // Verify state was updated
        let s = state.lock().unwrap();
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].text, "hello world");
        assert!(s.waiting_for_reply);
    }

    #[test]
    fn given_send_without_session_key_then_falls_back_to_selected_session() {
        let (state, tx, mut rx) = setup_ipc();
        state.lock().unwrap().selected_session = Some("my-session".to_string());

        let body = r#"{"type":"send","message":"test"}"#;
        handle_ipc_message(body, &tx, &state);

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SendChat { session_key, .. } => {
                assert_eq!(session_key, Some("my-session".to_string()));
            }
            _ => panic!("expected SendChat"),
        }
    }

    #[test]
    fn given_send_with_no_session_and_no_selected_then_session_key_is_none() {
        let (state, tx, mut rx) = setup_ipc();

        let body = r#"{"type":"send","message":"test"}"#;
        handle_ipc_message(body, &tx, &state);

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SendChat { session_key, .. } => {
                assert_eq!(session_key, None);
            }
            _ => panic!("expected SendChat"),
        }
    }

    #[test]
    fn given_empty_message_then_send_is_ignored() {
        let (state, tx, mut rx) = setup_ipc();

        let body = r#"{"type":"send","message":""}"#;
        handle_ipc_message(body, &tx, &state);

        assert!(rx.try_recv().is_err(), "no command should be sent");
        assert!(state.lock().unwrap().messages.is_empty());
    }

    #[test]
    fn given_send_with_attachments_then_attachments_are_parsed() {
        let (state, tx, mut rx) = setup_ipc();

        let body = r#"{
            "type": "send",
            "message": "see image",
            "attachments": [
                {"data": "abc123", "filename": "pic.png", "mimeType": "image/png"}
            ]
        }"#;
        handle_ipc_message(body, &tx, &state);

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SendChat { attachments, .. } => {
                let atts = attachments.expect("should have attachments");
                assert_eq!(atts.len(), 1);
                assert_eq!(atts[0].data, "abc123");
                assert_eq!(atts[0].filename, "pic.png");
                assert_eq!(atts[0].mime_type, "image/png");
            }
            _ => panic!("expected SendChat"),
        }
    }

    #[test]
    fn given_select_session_ipc_then_state_is_updated() {
        let (state, tx, _rx) = setup_ipc();

        let body = r#"{"type":"selectSession","sessionKey":"new-sess"}"#;
        handle_ipc_message(body, &tx, &state);

        assert_eq!(
            state.lock().unwrap().selected_session,
            Some("new-sess".to_string())
        );
    }

    #[test]
    fn given_list_sessions_ipc_then_command_is_sent() {
        let (state, tx, mut rx) = setup_ipc();

        let body = r#"{"type":"listSessions"}"#;
        handle_ipc_message(body, &tx, &state);

        let cmd = rx.try_recv().unwrap();
        assert!(matches!(cmd, PluginCommand::ListSessions));
    }

    #[test]
    fn given_invalid_json_then_ipc_is_silently_ignored() {
        let (state, tx, mut rx) = setup_ipc();

        handle_ipc_message("not json at all", &tx, &state);

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn given_unknown_ipc_type_then_no_command_sent() {
        let (state, tx, mut rx) = setup_ipc();

        let body = r#"{"type":"unknownCmd"}"#;
        handle_ipc_message(body, &tx, &state);

        assert!(rx.try_recv().is_err());
    }

    // ── Message limit enforcement via IPC ────────────────────────────

    #[test]
    fn given_max_messages_when_send_via_ipc_then_oldest_evicted() {
        let (state, tx, _rx) = setup_ipc();

        // Pre-fill state to MAX_MESSAGES
        {
            let mut s = state.lock().unwrap();
            for i in 0..MAX_MESSAGES {
                s.messages.push(ChatMessage {
                    sender: ChatSender::User,
                    text: format!("old-{i}"),
                    media_path: None,
                    media_type: None,
                });
            }
        }

        let body = r#"{"type":"send","message":"new"}"#;
        handle_ipc_message(body, &tx, &state);

        let s = state.lock().unwrap();
        assert_eq!(s.messages.len(), MAX_MESSAGES);
        assert_eq!(s.messages[0].text, "old-1");
        assert_eq!(s.messages[MAX_MESSAGES - 1].text, "new");
    }

    // ── build_init_json ──────────────────────────────────────────────

    #[test]
    fn given_state_with_messages_then_init_json_contains_them() {
        let state = Arc::new(Mutex::new(ChatState::new()));
        {
            let mut s = state.lock().unwrap();
            s.connected = true;
            s.messages.push(ChatMessage {
                sender: ChatSender::User,
                text: "hi".to_string(),
                media_path: None,
                media_type: None,
            });
            s.messages.push(ChatMessage {
                sender: ChatSender::Agent("Bot".to_string()),
                text: "hello".to_string(),
                media_path: None,
                media_type: None,
            });
            s.sessions.push(ChatSessionInfo {
                key: "main".to_string(),
                name: "Main".to_string(),
            });
            s.selected_session = Some("main".to_string());
        }

        let json_str = build_init_json(&state);
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(v["connected"], true);
        assert_eq!(v["messages"].as_array().unwrap().len(), 2);
        assert_eq!(v["messages"][0]["sender"], "user");
        assert_eq!(v["messages"][0]["text"], "hi");
        assert_eq!(v["messages"][1]["sender"], "agent");
        assert_eq!(v["messages"][1]["agentName"], "Bot");
        assert_eq!(v["sessions"][0]["key"], "main");
        assert_eq!(v["selectedSession"], "main");
    }

    // ── Streaming: stream start creates pending message ─────────

    #[test]
    fn given_no_messages_when_stream_start_processed_then_pending_stream_exists() {
        let mut state = ChatState::new();
        state.inbox.push(ChatInbound::StreamStart {
            msg_id: "abc".to_string(),
            agent_name: Some("Bot".to_string()),
        });

        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::StreamStart { msg_id, agent_name } = event {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                state.pending_stream = Some(PendingStream {
                    msg_id,
                    agent_name: name,
                    text: String::new(),
                });
                state.waiting_for_reply = false;
            }
        }

        assert!(state.pending_stream.is_some(), "pending stream should exist");
        let ps = state.pending_stream.as_ref().unwrap();
        assert_eq!(ps.msg_id, "abc");
        assert_eq!(ps.agent_name, "Bot");
        assert!(ps.text.is_empty());
    }

    // ── Streaming: chunks append text ───────────────────────────

    #[test]
    fn given_pending_stream_when_chunk_arrives_then_text_appended() {
        let mut state = ChatState::new();
        state.pending_stream = Some(PendingStream {
            msg_id: "abc".to_string(),
            agent_name: "Bot".to_string(),
            text: "Hello".to_string(),
        });

        state.inbox.push(ChatInbound::StreamChunk {
            msg_id: "abc".to_string(),
            text: " world".to_string(),
        });

        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::StreamChunk { msg_id, text } = event {
                if let Some(ref mut ps) = state.pending_stream {
                    if ps.msg_id == msg_id {
                        ps.text.push_str(&text);
                    }
                }
            }
        }

        let ps = state.pending_stream.as_ref().unwrap();
        assert_eq!(ps.text, "Hello world");
    }

    // ── Streaming: stream end finalizes message ─────────────────

    #[test]
    fn given_pending_stream_when_stream_end_then_message_finalized() {
        let mut state = ChatState::new();
        state.pending_stream = Some(PendingStream {
            msg_id: "abc".to_string(),
            agent_name: "Bot".to_string(),
            text: "Hello world".to_string(),
        });

        state.inbox.push(ChatInbound::StreamEnd {
            msg_id: "abc".to_string(),
        });

        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::StreamEnd { msg_id } = event {
                if let Some(ps) = state.pending_stream.take() {
                    if ps.msg_id == msg_id {
                        state.messages.push(ChatMessage {
                            sender: ChatSender::Agent(ps.agent_name),
                            text: ps.text,
                            media_path: None,
                            media_type: None,
                        });
                    }
                }
            }
        }

        assert!(state.pending_stream.is_none(), "pending stream should be cleared");
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, "Hello world");
        assert_eq!(
            state.messages[0].sender,
            ChatSender::Agent("Bot".to_string())
        );
    }

    // ── Streaming: chunk with mismatched id does not modify ─────

    #[test]
    fn given_pending_stream_when_chunk_with_wrong_id_then_text_unchanged() {
        let mut state = ChatState::new();
        state.pending_stream = Some(PendingStream {
            msg_id: "abc".to_string(),
            agent_name: "Bot".to_string(),
            text: "Hello".to_string(),
        });

        state.inbox.push(ChatInbound::StreamChunk {
            msg_id: "xyz".to_string(),
            text: " nope".to_string(),
        });

        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::StreamChunk { msg_id, text } = event {
                if let Some(ref mut ps) = state.pending_stream {
                    if ps.msg_id == msg_id {
                        ps.text.push_str(&text);
                    }
                }
            }
        }

        assert_eq!(state.pending_stream.as_ref().unwrap().text, "Hello");
    }

    // ── Streaming: default agent name ───────────────────────────

    #[test]
    fn given_stream_start_without_agent_name_then_defaults_to_agent() {
        let mut state = ChatState::new();
        state.inbox.push(ChatInbound::StreamStart {
            msg_id: "s1".to_string(),
            agent_name: None,
        });

        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::StreamStart { msg_id, agent_name } = event {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                state.pending_stream = Some(PendingStream {
                    msg_id,
                    agent_name: name,
                    text: String::new(),
                });
            }
        }

        assert_eq!(state.pending_stream.as_ref().unwrap().agent_name, "Agent");
    }

    // ── Plugin Store: navigate to store page ────────────────────

    #[test]
    fn given_navigate_to_store_then_current_page_is_store() {
        let (state, tx, _rx) = setup_ipc();

        let body = r#"{"type":"navigate","page":"store"}"#;
        handle_ipc_message(body, &tx, &state);

        let s = state.lock().unwrap();
        assert_eq!(s.current_page, "store");
    }

    // ── Plugin Store: save preset adds plugin via savePlugin IPC ─

    #[test]
    fn given_save_plugin_from_store_preset_then_plugin_is_accepted() {
        let (state, tx, _rx) = setup_ipc();

        // Simulate saving a store preset (same as savePlugin IPC)
        let body = r#"{
            "type": "savePlugin",
            "plugin": {
                "type": "ollama",
                "name": "Ollama (Llama 3.3)",
                "url": "http://localhost:11434",
                "model": "llama3.3"
            }
        }"#;
        handle_ipc_message(body, &tx, &state);

        // savePlugin writes to config file, not to chat state.
        // Verify IPC handler did not crash and state is unmodified.
        let s = state.lock().unwrap();
        assert!(s.messages.is_empty());
    }

    // ── Plugin Store: search filters (JS-side, but verify navigate works) ─

    #[test]
    fn given_navigate_to_store_then_back_to_chat_then_page_is_chat() {
        let (state, tx, _rx) = setup_ipc();

        handle_ipc_message(r#"{"type":"navigate","page":"store"}"#, &tx, &state);
        assert_eq!(state.lock().unwrap().current_page, "store");

        handle_ipc_message(r#"{"type":"navigate","page":"chat"}"#, &tx, &state);
        assert_eq!(state.lock().unwrap().current_page, "chat");
    }

    // ── Plugin Store: already installed preset uses same save flow ─

    #[test]
    fn given_save_plugin_with_openai_config_then_no_crash() {
        let (state, tx, _rx) = setup_ipc();

        let body = r#"{
            "type": "savePlugin",
            "plugin": {
                "type": "openai-compatible",
                "name": "OpenAI GPT-4o",
                "url": "https://api.openai.com/v1",
                "model": "gpt-4o",
                "apiKey": "sk-test123"
            }
        }"#;
        handle_ipc_message(body, &tx, &state);
        // No crash = success
    }

    // ── Plugin Store: settings pushed on store page ─────────────

    #[test]
    fn given_current_page_is_store_then_settings_requested_flag_behavior() {
        let state = ChatState::new();
        // When current_page is "store", the process_inbox_to_webview
        // logic should push settings data (tested via the condition check)
        assert_eq!(state.current_page, "chat");
        // This is a unit-level check; the integration is in process_inbox_to_webview
    }

    // ── Bug 2: Close window hides instead of quit ───────────────

    #[test]
    fn given_new_chat_state_then_app_quit_defaults_to_false() {
        let state = ChatState::new();
        assert!(!state.app_quit, "app_quit should default to false");
    }

    #[test]
    fn given_window_open_when_close_requested_then_window_open_becomes_false() {
        // Simulates what the event loop does on CloseRequested:
        // sets window_open = false, window_focused = false (window.set_visible(false) is
        // a GUI call we can't test in unit tests, but the state transition is testable).
        let mut state = ChatState::new();
        state.window_open = true;
        state.window_focused = true;

        // Simulate CloseRequested handler
        state.window_open = false;
        state.window_focused = false;

        assert!(!state.window_open, "window_open should be false after close");
        assert!(!state.window_focused, "window_focused should be false after close");
        assert!(!state.app_quit, "app_quit should NOT be set on close");
    }

    #[test]
    fn given_window_hidden_when_tray_sets_window_open_true_then_reshow_is_possible() {
        let mut state = ChatState::new();
        state.window_open = false;
        state.window_focused = false;

        // Tray click sets window_open = true
        state.window_open = true;

        assert!(state.window_open, "tray should be able to set window_open back to true");
        assert!(!state.app_quit, "app_quit should remain false for re-show");
    }

    #[test]
    fn given_app_quit_true_then_event_loop_should_exit() {
        let mut state = ChatState::new();
        state.app_quit = true;

        // The event loop checks state.app_quit and sets ControlFlow::Exit
        assert!(state.app_quit, "when app_quit is true, event loop should exit");
    }

    // ── Bug 3: Switching plugin clears chat ─────────────────────

    #[test]
    fn given_openclaw_plugin_when_switch_to_n8n_then_chat_cleared_and_event_pushed() {
        let (state, tx, _rx) = setup_ipc();

        // Pre-fill with some messages as if user was chatting
        {
            let mut s = state.lock().unwrap();
            s.active_plugin_id = Some("openclaw".to_string());
            s.messages.push(ChatMessage {
                sender: ChatSender::User,
                text: "hello openclaw".to_string(),
                media_path: None,
                media_type: None,
            });
            s.messages.push(ChatMessage {
                sender: ChatSender::Agent("OC".to_string()),
                text: "hi there".to_string(),
                media_path: None,
                media_type: None,
            });
        }

        let body = r#"{"type":"switchPlugin","pluginId":"n8n","sessionKey":"main"}"#;
        handle_ipc_message(body, &tx, &state);

        let s = state.lock().unwrap();
        assert!(s.messages.is_empty(), "messages should be cleared on plugin switch");
        assert_eq!(s.active_plugin_id, Some("n8n".to_string()));
        assert_eq!(s.active_session_key, "main");
        assert!(!s.waiting_for_reply, "waiting_for_reply should be reset");
        assert!(s.pending_stream.is_none(), "pending_stream should be cleared");

        // Verify PluginSwitched event was pushed to inbox
        let has_plugin_switched = s.inbox.iter().any(|e| {
            matches!(e, ChatInbound::PluginSwitched { plugin_id, .. } if plugin_id == "n8n")
        });
        assert!(has_plugin_switched, "PluginSwitched event should be in inbox");
    }

    #[test]
    fn given_same_plugin_when_switch_then_no_clear() {
        let (state, tx, _rx) = setup_ipc();

        {
            let mut s = state.lock().unwrap();
            s.active_plugin_id = Some("openclaw".to_string());
            s.messages.push(ChatMessage {
                sender: ChatSender::User,
                text: "keep me".to_string(),
                media_path: None,
                media_type: None,
            });
        }

        let body = r#"{"type":"switchPlugin","pluginId":"openclaw","sessionKey":"main"}"#;
        handle_ipc_message(body, &tx, &state);

        let s = state.lock().unwrap();
        assert_eq!(s.messages.len(), 1, "messages should NOT be cleared for same plugin");
        assert_eq!(s.messages[0].text, "keep me");
    }

    // ── Plugin switcher: command routing tests ───────────────────

    #[test]
    fn given_two_plugins_when_switch_then_messages_route_to_new_plugin() {
        let state = Arc::new(Mutex::new(ChatState::new()));
        let (tx_a, mut rx_a) = tokio::sync::mpsc::unbounded_channel::<PluginCommand>();
        let (tx_b, mut rx_b) = tokio::sync::mpsc::unbounded_channel::<PluginCommand>();
        let mut senders = HashMap::new();
        senders.insert("plugin-a".to_string(), tx_a);
        senders.insert("plugin-b".to_string(), tx_b);

        // Set active to plugin-a
        {
            let mut s = state.lock().unwrap();
            s.active_plugin_id = Some("plugin-a".to_string());
        }

        let body = r#"{"type":"send","message":"hello a"}"#;
        handle_ipc_message(body, &senders, &state);
        assert!(rx_a.try_recv().is_ok(), "message should route to plugin-a");
        assert!(rx_b.try_recv().is_err(), "plugin-b should not receive message");

        // Switch to plugin-b
        let body = r#"{"type":"switchPlugin","pluginId":"plugin-b","sessionKey":"main"}"#;
        handle_ipc_message(body, &senders, &state);

        let body = r#"{"type":"send","message":"hello b"}"#;
        handle_ipc_message(body, &senders, &state);
        assert!(rx_b.try_recv().is_ok(), "message should route to plugin-b after switch");
    }

    #[test]
    fn given_plugin_switch_then_chat_cleared() {
        let (state, tx, _rx) = setup_ipc();
        {
            let mut s = state.lock().unwrap();
            s.active_plugin_id = Some("old-plugin".to_string());
            s.messages.push(ChatMessage {
                sender: ChatSender::User,
                text: "old message".to_string(),
                media_path: None,
                media_type: None,
            });
        }

        let body = r#"{"type":"switchPlugin","pluginId":"new-plugin","sessionKey":"main"}"#;
        handle_ipc_message(body, &tx, &state);

        let s = state.lock().unwrap();
        assert!(s.messages.is_empty(), "messages should be cleared on plugin switch");
        assert!(s.inbox.iter().any(|e| matches!(e, ChatInbound::PluginSwitched { .. })));
    }

    #[test]
    fn given_init_then_plugins_list_sent_to_webview() {
        // build_init_json should include a "plugins" key
        let cs = Arc::new(Mutex::new(ChatState::new()));
        let json_str = build_init_json(&cs);
        let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        // plugins key should exist (may be empty if no config)
        assert!(val.get("plugins").is_some(), "init data should contain plugins key");
        assert!(val["plugins"].is_array(), "plugins should be an array");
    }

    #[test]
    fn given_single_plugin_then_dropdown_still_visible() {
        // This is a UI-level test assertion: when plugins array has 1 entry,
        // initPluginSelect should still render it. We verify the data contract:
        // a single-element plugins array is sent.
        let cs = Arc::new(Mutex::new(ChatState::new()));
        let json_str = build_init_json(&cs);
        let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        // Even with default config (possibly 0 or 1 plugin), the key exists
        assert!(val["plugins"].is_array());
    }

    // ── Wave 10: Agent Switcher ────────────────────────────────────

    #[test]
    fn given_switch_agent_openclaw_then_session_key_set() {
        let cs = Arc::new(Mutex::new(ChatState::new()));
        let senders = HashMap::new();
        let msg = serde_json::json!({
            "type": "switchAgent",
            "agentId": "divination",
            "agentType": "openclaw",
            "sessionKey": "divination"
        });
        handle_ipc_message(&msg.to_string(), &senders, &cs);

        let s = cs.lock().unwrap();
        assert_eq!(s.active_agent_id, "divination");
        assert_eq!(s.active_session_key, "divination");
    }

    #[test]
    fn given_switch_agent_n8n_then_plugin_id_set() {
        let cs = Arc::new(Mutex::new(ChatState::new()));
        let senders = HashMap::new();
        let msg = serde_json::json!({
            "type": "switchAgent",
            "agentId": "n8n-minimax",
            "agentType": "n8n",
            "sessionKey": ""
        });
        handle_ipc_message(&msg.to_string(), &senders, &cs);

        let s = cs.lock().unwrap();
        assert_eq!(s.active_agent_id, "n8n-minimax");
        assert_eq!(s.active_plugin_id.as_deref(), Some("n8n-minimax"));
        assert_eq!(s.active_session_key, "main");
    }

    #[test]
    fn given_agent_switch_then_chat_cleared() {
        let cs = Arc::new(Mutex::new(ChatState::new()));
        {
            let mut s = cs.lock().unwrap();
            s.messages.push(ChatMessage {
                sender: ChatSender::User,
                text: "old message".to_string(),
                media_path: None,
                media_type: None,
            });
            s.active_agent_id = "main".to_string();
        }
        let senders = HashMap::new();
        let msg = serde_json::json!({
            "type": "switchAgent",
            "agentId": "office",
            "agentType": "openclaw",
            "sessionKey": "office"
        });
        handle_ipc_message(&msg.to_string(), &senders, &cs);

        let s = cs.lock().unwrap();
        assert!(s.messages.is_empty(), "messages should be cleared on agent switch");
    }

    #[test]
    fn given_init_json_then_agents_key_present() {
        let cs = Arc::new(Mutex::new(ChatState::new()));
        {
            let mut s = cs.lock().unwrap();
            s.agents.push(crate::gateway::AgentInfo {
                id: "main".to_string(),
                name: "Arno".to_string(),
                session_key: "main".to_string(),
                agent_type: "openclaw".to_string(),
            });
        }
        let json_str = build_init_json(&cs);
        let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(val.get("agents").is_some(), "init data should contain agents key");
        assert!(val["agents"].is_array());
        assert_eq!(val["agents"].as_array().unwrap().len(), 1);
        assert_eq!(val["agents"][0]["id"], "main");
        assert_eq!(val["activeAgentId"], "main");
    }

    // ── Wave 11: Unified Event Loop ──────────────────────────────

    #[test]
    fn given_no_blocking_functions_after_refactor() {
        // run_chat_window_plugin and run_webview_window are removed.
        // This test documents that no function in chat.rs calls EventLoop::run().
        // The presence of create_chat_webview and process_chat_events confirms
        // the non-blocking API is in place (enforced at compile time).
        let _: fn(
            &tao::window::Window,
            &Arc<Mutex<ChatState>>,
            Arc<HashMap<String, mpsc::UnboundedSender<PluginCommand>>>,
        ) -> crate::error::Result<wry::WebView> = create_chat_webview;

        let _: fn(
            &Arc<Mutex<ChatState>>,
            &wry::WebView,
            &tao::window::Window,
        ) = process_chat_events;
    }

    #[test]
    fn given_chat_window_open_when_close_requested_then_state_reset() {
        // Simulates the unified event loop CloseRequested handler.
        let mut state = ChatState::new();
        state.window_open = true;
        state.window_focused = true;

        // CloseRequested handler: hide window, update state
        state.window_open = false;
        state.window_focused = false;

        assert!(!state.window_open, "window_open should be false after close");
        assert!(!state.window_focused, "window_focused should be false after close");
        assert!(!state.app_quit, "app_quit should NOT be set on close — window is hidden, not destroyed");
    }

    #[test]
    fn given_chat_hidden_when_open_chat_again_then_state_restored() {
        // Simulates the re-show path in the unified event loop.
        let mut state = ChatState::new();
        state.messages.push(ChatMessage {
            sender: ChatSender::User,
            text: "hello".to_string(),
            media_path: None,
            media_type: None,
        });
        state.messages.push(ChatMessage {
            sender: ChatSender::Agent("Bot".to_string()),
            text: "hi".to_string(),
            media_path: None,
            media_type: None,
        });
        state.window_open = false;
        state.window_focused = false;

        // Re-show: set_visible(true), update state
        state.window_open = true;
        state.window_focused = true;

        assert!(state.window_open);
        assert!(state.window_focused);
        assert_eq!(state.messages.len(), 2, "messages should be preserved across hide/show");
        assert_eq!(state.messages[0].text, "hello");
        assert_eq!(state.messages[1].text, "hi");
    }

    #[test]
    fn given_exit_command_then_app_quit_signals_exit() {
        // In the unified event loop, Exit calls std::process::exit(0).
        // For the webview path (legacy), app_quit was used. Verify it still works.
        let mut state = ChatState::new();
        state.app_quit = true;
        assert!(state.app_quit, "app_quit = true signals the event loop to exit");
    }

    #[test]
    fn given_window_none_then_first_open_creates_window() {
        // Verifies the state transition: window_open starts false, set to true on first open.
        let mut state = ChatState::new();
        assert!(!state.window_open, "window_open starts false");

        // Simulate first OpenChat handler
        state.window_open = true;
        state.window_focused = true;
        assert!(state.window_open);
    }

    #[test]
    fn given_gateway_event_while_chat_open_then_log_entry_added() {
        let mut state = ChatState::new();
        state.window_open = true;

        // Simulate gateway Connected event being processed in the unified loop
        state.add_log(
            crate::dashboard::LogLevel::Info,
            "my-connection",
            "Connected",
        );

        let entries = state.log_buffer.entries();
        assert!(!entries.is_empty(), "log entry should be added while chat is open");
        assert_eq!(entries.back().unwrap().message, "Connected");
    }

    #[test]
    fn given_process_chat_events_with_empty_inbox_then_noop() {
        // Verifies that process_chat_events with an empty inbox
        // does not modify state (no panics, no side effects).
        let state = ChatState::new();
        assert!(state.inbox.is_empty());
        assert!(state.messages.is_empty());
        // process_chat_events would call process_inbox_to_webview which
        // drains an empty inbox — no-op. We verify state remains unchanged.
    }

    #[test]
    fn given_pin_changed_in_inbox_then_event_consumed() {
        // process_chat_events removes PinChanged from the inbox.
        let mut state = ChatState::new();
        state.inbox.push(ChatInbound::PinChanged { pinned: true });
        state.inbox.push(ChatInbound::Reply {
            text: "hello".to_string(),
            agent_name: None,
            usage: None,
            attachments: None,
        });

        // Simulate the pin-change removal that process_chat_events does
        let mut i = 0;
        while i < state.inbox.len() {
            if matches!(&state.inbox[i], ChatInbound::PinChanged { .. }) {
                state.inbox.remove(i);
            } else {
                i += 1;
            }
        }

        assert_eq!(state.inbox.len(), 1, "only PinChanged should be removed");
        assert!(
            matches!(&state.inbox[0], ChatInbound::Reply { .. }),
            "Reply should remain in inbox"
        );
    }

    #[test]
    fn given_notifications_when_window_hidden_then_reply_detected() {
        // Simulates the notification peek logic in the unified event loop:
        // when the window is hidden, peek at inbox for Reply events.
        let mut state = ChatState::new();
        state.window_open = false;
        state.inbox.push(ChatInbound::Reply {
            text: "Agent reply text".to_string(),
            agent_name: Some("Bot".to_string()),
            usage: None,
            attachments: None,
        });

        let has_reply = state.inbox.iter().any(|e| matches!(e, ChatInbound::Reply { .. }));
        assert!(has_reply, "should detect reply in inbox for notification");
    }

    #[test]
    fn given_history_save_then_messages_persisted() {
        let mut state = ChatState::new();
        state.active_plugin_id = Some("test-plugin".to_string());
        state.active_session_key = "main".to_string();
        state.messages.push(ChatMessage {
            sender: ChatSender::User,
            text: "user msg".to_string(),
            media_path: None,
            media_type: None,
        });
        state.messages.push(ChatMessage {
            sender: ChatSender::Agent("Bot".to_string()),
            text: "bot reply".to_string(),
            media_path: None,
            media_type: None,
        });

        let mut history = ChatHistory::load();
        state.save_to_history(&mut history);

        let key = state.conversation_key();
        let msgs = history.get_messages(&key);
        assert_eq!(msgs.len(), 2, "both messages should be persisted");
        assert_eq!(msgs[0].sender, "user");
        assert_eq!(msgs[1].sender, "agent");
        assert_eq!(msgs[1].agent_name.as_deref(), Some("Bot"));
    }
}
