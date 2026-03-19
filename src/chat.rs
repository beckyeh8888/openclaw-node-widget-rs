use std::sync::{Arc, Mutex};

use eframe::egui;
use tokio::sync::mpsc;

use crate::gateway::{ChatSessionInfo, GatewayCommand};
use crate::i18n::{setup_cjk_fonts, t};

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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([400.0, 500.0]),
        ..Default::default()
    };

    let state_for_app = Arc::clone(&chat_state);
    let result = eframe::run_native(
        t("chat"),
        options,
        Box::new(
            move |_cc| -> std::result::Result<
                Box<dyn eframe::App>,
                Box<dyn std::error::Error + Send + Sync>,
            > {
                Ok(Box::new(ChatApp {
                    chat_state: state_for_app,
                    cmd_tx,
                    input: String::new(),
                    dark_mode_applied: false,
                }))
            },
        ),
    );

    if let Ok(mut state) = chat_state.lock() {
        state.window_open = false;
        state.window_focused = false;
    }

    result.map_err(|e| crate::error::AppError::Tray(format!("chat window: {e}")))
}

struct ChatApp {
    chat_state: Arc<Mutex<ChatState>>,
    cmd_tx: mpsc::UnboundedSender<GatewayCommand>,
    input: String,
    dark_mode_applied: bool,
}

impl ChatApp {
    fn process_inbox(&self) {
        let Ok(mut state) = self.chat_state.lock() else {
            return;
        };
        let events: Vec<_> = state.inbox.drain(..).collect();
        for event in events {
            match event {
                ChatInbound::Reply { text, agent_name } => {
                    let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                    state.messages.push(ChatMessage {
                        sender: ChatSender::Agent(name),
                        text,
                    });
                    state.waiting_for_reply = false;
                    while state.messages.len() > MAX_MESSAGES {
                        state.messages.remove(0);
                    }
                }
                ChatInbound::SessionsList { sessions } => {
                    state.sessions = sessions;
                }
                ChatInbound::Connected => {
                    state.connected = true;
                }
                ChatInbound::Disconnected => {
                    state.connected = false;
                    state.waiting_for_reply = false;
                }
            }
        }
    }

    fn send_message(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }

        let session_key = self
            .chat_state
            .lock()
            .ok()
            .and_then(|s| s.selected_session.clone());

        let _ = self.cmd_tx.send(GatewayCommand::SendChat {
            message: text.clone(),
            session_key,
        });

        if let Ok(mut state) = self.chat_state.lock() {
            state.messages.push(ChatMessage {
                sender: ChatSender::User,
                text,
            });
            state.waiting_for_reply = true;
            while state.messages.len() > MAX_MESSAGES {
                state.messages.remove(0);
            }
        }

        self.input.clear();
    }
}

impl eframe::App for ChatApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.dark_mode_applied {
            ctx.set_visuals(egui::Visuals::dark());
            setup_cjk_fonts(ctx);
            self.dark_mode_applied = true;
        }

        self.process_inbox();
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        // Snapshot shared state for this frame
        let (messages, waiting, connected, selected_session, sessions) = {
            let s = self.chat_state.lock().unwrap_or_else(|e| e.into_inner());
            (
                s.messages.clone(),
                s.waiting_for_reply,
                s.connected,
                s.selected_session.clone(),
                s.sessions.clone(),
            )
        };

        // Header with agent selector
        egui::TopBottomPanel::top("chat_header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(t("chat"));
                ui.separator();

                let selected_label = selected_session
                    .as_ref()
                    .and_then(|key| sessions.iter().find(|s| &s.key == key))
                    .map(|s| s.name.as_str())
                    .unwrap_or("main");

                egui::ComboBox::from_id_salt("agent_select")
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        let mut sel = selected_session.clone();
                        if ui.selectable_value(&mut sel, None, "main").clicked() {
                            if let Ok(mut s) = self.chat_state.lock() {
                                s.selected_session = None;
                            }
                        }
                        for session in &sessions {
                            if ui
                                .selectable_value(
                                    &mut sel,
                                    Some(session.key.clone()),
                                    &session.name,
                                )
                                .clicked()
                            {
                                if let Ok(mut s) = self.chat_state.lock() {
                                    s.selected_session = Some(session.key.clone());
                                }
                            }
                        }
                    });

                if !connected {
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.colored_label(
                                egui::Color32::from_rgb(220, 80, 80),
                                t("chat_not_connected"),
                            );
                        },
                    );
                }
            });
        });

        // Input area at bottom
        egui::TopBottomPanel::bottom("chat_input").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let response = ui.add_sized(
                    [ui.available_width() - 60.0, 24.0],
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text(t("chat_placeholder"))
                        .interactive(connected),
                );

                let enter_pressed =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let can_send = connected && !self.input.trim().is_empty();

                if ui
                    .add_enabled(can_send, egui::Button::new(t("chat_send")))
                    .clicked()
                    || (enter_pressed && can_send)
                {
                    self.send_message();
                    response.request_focus();
                }
            });
            ui.add_space(4.0);
        });

        // Scrollable message area
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if messages.is_empty() && !waiting {
                        ui.centered_and_justified(|ui| {
                            ui.label(t("chat_empty"));
                        });
                    } else {
                        for msg in &messages {
                            match &msg.sender {
                                ChatSender::User => {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::TOP),
                                        |ui| {
                                            ui.group(|ui| {
                                                ui.set_max_width(ui.available_width() * 0.75);
                                                ui.label(&msg.text);
                                            });
                                        },
                                    );
                                }
                                ChatSender::Agent(name) => {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(100, 180, 255),
                                        format!("{name}:"),
                                    );
                                    ui.group(|ui| {
                                        ui.set_max_width(ui.available_width() * 0.75);
                                        ui.label(&msg.text);
                                    });
                                }
                            }
                            ui.add_space(4.0);
                        }
                        if waiting {
                            ui.colored_label(
                                egui::Color32::from_rgb(150, 150, 150),
                                t("chat_typing"),
                            );
                        }
                    }
                });
        });
    }
}
