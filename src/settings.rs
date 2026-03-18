use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::{
    autostart,
    config::{Config, ConnectionConfig},
    error::{AppError, Result},
    i18n::t,
};

struct SharedState {
    saved_config: Option<Config>,
}

pub fn run_settings_window(config: &Config) -> Result<Option<Config>> {
    let shared = Arc::new(Mutex::new(SharedState {
        saved_config: None,
    }));
    let shared_clone = Arc::clone(&shared);
    let initial = config.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([500.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        t("settings_title"),
        options,
        Box::new(
            move |_cc| -> std::result::Result<
                Box<dyn eframe::App>,
                Box<dyn std::error::Error + Send + Sync>,
            > {
                Ok(Box::new(SettingsApp::new(initial, shared_clone)))
            },
        ),
    )
    .map_err(|e| AppError::Tray(format!("settings window failed: {e}")))?;

    let saved = shared
        .lock()
        .map_err(|_| AppError::Config("settings state lock poisoned".to_string()))?
        .saved_config
        .clone();

    Ok(saved)
}

struct ConnectionEdit {
    name: String,
    gateway_url: String,
    gateway_token: String,
}

struct SettingsApp {
    shared: Arc<Mutex<SharedState>>,
    connections: Vec<ConnectionEdit>,
    auto_restart: bool,
    auto_start: bool,
    check_interval: f32,
    notifications: bool,
    notification_sound: bool,
    language_idx: usize,
    save_message: Option<String>,
    save_error: Option<String>,
    base_config: Config,
    dark_mode_applied: bool,
}

impl SettingsApp {
    fn new(config: Config, shared: Arc<Mutex<SharedState>>) -> Self {
        let effective = config.effective_connections();
        let connections: Vec<ConnectionEdit> = if effective.is_empty() {
            // Show one empty connection editor
            vec![ConnectionEdit {
                name: "Default".to_string(),
                gateway_url: String::new(),
                gateway_token: String::new(),
            }]
        } else {
            effective
                .iter()
                .map(|c| ConnectionEdit {
                    name: c.name.clone(),
                    gateway_url: c.gateway_url.clone(),
                    gateway_token: c.gateway_token.clone().unwrap_or_default(),
                })
                .collect()
        };

        Self {
            auto_restart: config.widget.auto_restart,
            auto_start: autostart::effective_autostart(&config),
            check_interval: config.widget.check_interval_secs as f32,
            notifications: config.widget.notifications,
            notification_sound: config.widget.notification_sound,
            language_idx: crate::i18n::LANGUAGE_OPTIONS
                .iter()
                .position(|(code, _)| *code == config.widget.language)
                .unwrap_or(0),
            save_message: None,
            save_error: None,
            base_config: config,
            dark_mode_applied: false,
            shared,
            connections,
        }
    }

    fn save_settings(&mut self) {
        self.save_message = None;
        self.save_error = None;

        let mut config = self.base_config.clone();

        // Build connections from editor state
        let connections: Vec<ConnectionConfig> = self
            .connections
            .iter()
            .filter(|c| !c.gateway_url.trim().is_empty())
            .map(|c| ConnectionConfig {
                name: if c.name.trim().is_empty() {
                    "Default".to_string()
                } else {
                    c.name.trim().to_string()
                },
                gateway_url: c.gateway_url.trim().to_string(),
                gateway_token: if c.gateway_token.trim().is_empty() {
                    None
                } else {
                    Some(c.gateway_token.trim().to_string())
                },
            })
            .collect();

        config.connections = connections;
        // Clear old-style gateway fields
        config.gateway.url = None;
        config.gateway.token = None;

        config.widget.auto_restart = self.auto_restart;
        config.widget.check_interval_secs = self.check_interval as u64;
        config.widget.notifications = self.notifications;
        config.widget.notification_sound = self.notification_sound;
        config.widget.language = crate::i18n::LANGUAGE_OPTIONS[self.language_idx].0.to_string();
        crate::i18n::set_language(&config.widget.language);
        config.startup.auto_start = self.auto_start;

        if let Err(err) = autostart::set_autostart(self.auto_start) {
            self.save_error = Some(format!("Autostart error: {err}"));
            return;
        }

        if let Err(err) = config.save() {
            self.save_error = Some(format!("Save error: {err}"));
            return;
        }

        if let Ok(mut state) = self.shared.lock() {
            state.saved_config = Some(config.clone());
        }

        self.base_config = config;
        self.save_message = Some(t("settings_saved").to_string());
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.dark_mode_applied {
            ctx.set_visuals(egui::Visuals::dark());
            crate::i18n::setup_cjk_fonts(ctx);
            self.dark_mode_applied = true;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(t("settings_title"));
            ui.add_space(8.0);

            // Connections section
            ui.label(t("connections_label"));
            ui.add_space(4.0);

            let conn_count = self.connections.len();
            let mut remove_idx = None;
            for i in 0..conn_count {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(t("connection_name"));
                        ui.text_edit_singleline(&mut self.connections[i].name);
                        if conn_count > 1 {
                            if ui.button(t("remove")).clicked() {
                                remove_idx = Some(i);
                            }
                        }
                    });
                    egui::Grid::new(format!("conn_grid_{i}"))
                        .num_columns(2)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            ui.label(t("gateway_url"));
                            ui.text_edit_singleline(&mut self.connections[i].gateway_url);
                            ui.end_row();

                            ui.label(t("gateway_token"));
                            ui.text_edit_singleline(&mut self.connections[i].gateway_token);
                            ui.end_row();
                        });
                });
                ui.add_space(4.0);
            }

            if let Some(idx) = remove_idx {
                self.connections.remove(idx);
            }

            if ui.button(t("add_connection")).clicked() {
                self.connections.push(ConnectionEdit {
                    name: format!("Connection {}", self.connections.len() + 1),
                    gateway_url: String::new(),
                    gateway_token: String::new(),
                });
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label(t("check_interval"));
                    ui.add(
                        egui::Slider::new(&mut self.check_interval, 5.0..=120.0)
                            .step_by(1.0)
                            .suffix(" s"),
                    );
                    ui.end_row();

                    ui.label(t("auto_restart"));
                    ui.checkbox(&mut self.auto_restart, "");
                    ui.end_row();

                    ui.label(t("auto_start"));
                    ui.checkbox(&mut self.auto_start, "");
                    ui.end_row();

                    ui.label(t("notifications"));
                    ui.checkbox(&mut self.notifications, "");
                    ui.end_row();

                    ui.label(t("notification_sound"));
                    ui.checkbox(&mut self.notification_sound, "");
                    ui.end_row();

                    ui.label("Language");
                    egui::ComboBox::from_id_salt("lang_combo")
                        .selected_text(crate::i18n::LANGUAGE_OPTIONS[self.language_idx].1)
                        .show_ui(ui, |ui| {
                            for (i, (_code, label)) in crate::i18n::LANGUAGE_OPTIONS.iter().enumerate() {
                                ui.selectable_value(&mut self.language_idx, i, *label);
                            }
                        });
                    ui.end_row();
                });

            ui.add_space(16.0);

            ui.horizontal(|ui| {
                if ui.button(t("save")).clicked() {
                    self.save_settings();
                }
                if ui.button(t("close")).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            if let Some(msg) = &self.save_message {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::from_rgb(80, 200, 80), msg);
            }
            if let Some(err) = &self.save_error {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
            }
        });
    }
}
