use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::mpsc;
use tracing::warn;

use crate::gateway::{ChatAttachment, ChatSessionInfo, GatewayCommand};
use crate::i18n;

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
    SessionsList {
        sessions: Vec<ChatSessionInfo>,
    },
    Connected,
    Disconnected,
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
        }
    }
}

pub fn run_chat_window(
    chat_state: Arc<Mutex<ChatState>>,
    cmd_tx: mpsc::UnboundedSender<GatewayCommand>,
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

    let _ = cmd_tx.send(GatewayCommand::ListSessions);

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
    cmd_tx: mpsc::UnboundedSender<GatewayCommand>,
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

    json!({
        "lang": lang,
        "connected": state.connected,
        "messages": messages,
        "sessions": sessions,
        "selectedSession": state.selected_session,
        "waitingForReply": state.waiting_for_reply,
    })
    .to_string()
}

fn handle_ipc_message(
    body: &str,
    cmd_tx: &mpsc::UnboundedSender<GatewayCommand>,
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
            if message.is_empty() {
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

            let _ = cmd_tx.send(GatewayCommand::SendChat {
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
        "listSessions" => {
            let _ = cmd_tx.send(GatewayCommand::ListSessions);
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
}
