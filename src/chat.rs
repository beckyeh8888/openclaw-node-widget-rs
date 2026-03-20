use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::mpsc;
use tracing::warn;

use crate::config::{Config, GeneralSettings, PluginConfig};
use crate::dashboard::{DashboardData, LogBuffer, LogEntry, LogLevel};
use crate::gateway::{ChatAttachment, ChatSessionInfo, GatewayCommand};
use crate::history::{ChatHistory, PersistedMessage};
use crate::i18n;
use crate::plugin::PluginCommand;

const MAX_MESSAGES: usize = 50;

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub sender: ChatSender,
    pub text: String,
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
}

#[derive(Debug, Clone)]
pub struct PendingStream {
    pub msg_id: String,
    pub agent_name: String,
    pub text: String,
}

#[derive(Debug)]
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

pub fn run_chat_window(
    chat_state: Arc<Mutex<ChatState>>,
    cmd_tx: mpsc::UnboundedSender<GatewayCommand>,
) -> crate::error::Result<()> {
    // Wrap the GatewayCommand sender in a PluginCommand sender for
    // backward compatibility with code that still uses GatewayCommand.
    let (plugin_tx, mut plugin_rx) = mpsc::unbounded_channel::<PluginCommand>();
    let gw_tx = cmd_tx.clone();
    tokio::spawn(async move {
        while let Some(cmd) = plugin_rx.recv().await {
            match cmd {
                PluginCommand::SendChat {
                    message,
                    session_key,
                    attachments,
                } => {
                    let _ = gw_tx.send(GatewayCommand::SendChat {
                        message,
                        session_key,
                        attachments,
                    });
                }
                PluginCommand::ListSessions => {
                    let _ = gw_tx.send(GatewayCommand::ListSessions);
                }
            }
        }
    });
    run_chat_window_plugin(chat_state, plugin_tx)
}

/// Open the chat window routing commands through the plugin system.
pub fn run_chat_window_plugin(
    chat_state: Arc<Mutex<ChatState>>,
    cmd_tx: mpsc::UnboundedSender<PluginCommand>,
) -> crate::error::Result<()> {
    if let Ok(state) = chat_state.lock() {
        if state.window_open {
            return Ok(());
        }
    }

    if let Ok(mut state) = chat_state.lock() {
        state.window_open = true;
        state.window_focused = true;
    }

    let _ = cmd_tx.send(PluginCommand::ListSessions);

    // Build init data and embed into HTML
    let init_json = build_init_json(&chat_state);
    let html_template = include_str!("chat_ui.html");
    let html = html_template.replace("\"__INIT_DATA__\"", &init_json);

    let result = run_webview_window(html, chat_state.clone(), cmd_tx);

    // Ensure cleanup on exit
    if let Ok(mut state) = chat_state.lock() {
        state.window_open = false;
        state.window_focused = false;
    }

    result
}

fn run_webview_window(
    html: String,
    chat_state: Arc<Mutex<ChatState>>,
    cmd_tx: mpsc::UnboundedSender<PluginCommand>,
) -> crate::error::Result<()> {
    use tao::dpi::LogicalSize;
    use tao::event::{Event, StartCause, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;

    let event_loop = EventLoopBuilder::new().build();

    let window = WindowBuilder::new()
        .with_title("\u{1f916} OpenClaw Chat")
        .with_inner_size(LogicalSize::new(420.0, 620.0))
        .with_min_inner_size(LogicalSize::new(380.0, 400.0))
        .build(&event_loop)
        .map_err(|e| crate::error::AppError::Tray(format!("chat window: {e}")))?;

    let cmd_tx_ipc = cmd_tx.clone();
    let chat_state_ipc = Arc::clone(&chat_state);

    let webview = wry::WebViewBuilder::new()
        .with_html(html)
        .with_ipc_handler(move |req| {
            let body = req.body();
            handle_ipc_message(body, &cmd_tx_ipc, &chat_state_ipc);
        })
        .build(&window)
        .map_err(|e| crate::error::AppError::Tray(format!("webview: {e}")))?;

    let chat_state_loop = Arc::clone(&chat_state);

    event_loop.run(move |event, _, control_flow| {
        let next_poll =
            std::time::Instant::now() + std::time::Duration::from_millis(200);
        *control_flow = ControlFlow::WaitUntil(next_poll);

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if let Ok(mut state) = chat_state_loop.lock() {
                    state.window_open = false;
                    state.window_focused = false;
                }
                *control_flow = ControlFlow::Exit;
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. })
            | Event::MainEventsCleared => {
                process_inbox_to_webview(&chat_state_loop, &webview);
            }
            _ => {}
        }
    });
}

fn build_init_json(chat_state: &Arc<Mutex<ChatState>>) -> String {
    let state = chat_state.lock().unwrap_or_else(|e| e.into_inner());

    let lang = match i18n::current_lang() {
        i18n::Lang::En => "en",
        i18n::Lang::ZhTw => "zh-tw",
        i18n::Lang::ZhCn => "zh-cn",
    };

    let messages: Vec<serde_json::Value> = state
        .messages
        .iter()
        .map(|m| match &m.sender {
            ChatSender::User => json!({"sender": "user", "text": m.text}),
            ChatSender::Agent(name) => {
                json!({"sender": "agent", "agentName": name, "text": m.text})
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

    let tts_config = Config::load().map(|c| c.tts).unwrap_or_default();

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
    cmd_tx: &mpsc::UnboundedSender<PluginCommand>,
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
                .map(String::from)
                .or_else(|| {
                    chat_state
                        .lock()
                        .ok()
                        .and_then(|s| s.selected_session.clone())
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

            let _ = cmd_tx.send(PluginCommand::SendChat {
                message: message.clone(),
                session_key,
                attachments,
            });

            if let Ok(mut state) = chat_state.lock() {
                state.messages.push(ChatMessage {
                    sender: ChatSender::User,
                    text: message,
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
                    state.active_plugin_id = Some(plugin_id);
                    state.active_session_key = session_key.clone();
                    state.selected_session = Some(session_key);
                }
            }
        }
        "listSessions" => {
            let _ = cmd_tx.send(PluginCommand::ListSessions);
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
            let _prompt = msg.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
            // System prompt is stored per-plugin; currently a no-op placeholder.
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
        "saveGeneral" => {
            let general = GeneralSettings {
                language: msg.get("language").and_then(|v| v.as_str()).unwrap_or("auto").to_string(),
                auto_start: msg.get("autoStart").and_then(|v| v.as_bool()).unwrap_or(false),
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
            ChatInbound::Reply { text, agent_name } => {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                state.messages.push(ChatMessage {
                    sender: ChatSender::Agent(name.clone()),
                    text: text.clone(),
                });
                state.waiting_for_reply = false;
                while state.messages.len() > MAX_MESSAGES {
                    state.messages.remove(0);
                }

                let msg_json = json!({
                    "sender": "agent",
                    "agentName": name,
                    "text": text,
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
                        ps.text.push_str(&text);
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

    // Push settings data to WebView if requested
    if state.settings_requested || state.current_page == "settings" {
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
                        })
                    })
                    .collect();
                let settings_data = json!({
                    "plugins": plugins_json,
                    "general": {
                        "language": config.widget.language,
                        "autoStart": config.startup.auto_start,
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
            });
        }
        assert_eq!(state.messages.len(), MAX_MESSAGES);

        // Simulate what handle_ipc_message does: push + evict
        state.messages.push(ChatMessage {
            sender: ChatSender::User,
            text: "overflow".to_string(),
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
        });

        // Simulate process_inbox_to_webview logic (without webview)
        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::Reply { text, agent_name } = event {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                state.messages.push(ChatMessage {
                    sender: ChatSender::Agent(name),
                    text,
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
        });

        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            if let ChatInbound::Reply { text, agent_name } = event {
                let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                state.messages.push(ChatMessage {
                    sender: ChatSender::Agent(name),
                    text,
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
        tokio::sync::mpsc::UnboundedSender<PluginCommand>,
        tokio::sync::mpsc::UnboundedReceiver<PluginCommand>,
    ) {
        let state = Arc::new(Mutex::new(ChatState::new()));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (state, tx, rx)
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
            });
            s.messages.push(ChatMessage {
                sender: ChatSender::Agent("Bot".to_string()),
                text: "hello".to_string(),
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
}
