use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::{
    autostart,
    config::Config,
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
        viewport: egui::ViewportBuilder::default().with_inner_size([460.0, 380.0]),
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

struct SettingsApp {
    shared: Arc<Mutex<SharedState>>,
    gateway_url: String,
    gateway_token: String,
    auto_restart: bool,
    auto_start: bool,
    check_interval: f32,
    notifications: bool,
    notification_sound: bool,
    save_message: Option<String>,
    save_error: Option<String>,
    base_config: Config,
    dark_mode_applied: bool,
}

impl SettingsApp {
    fn new(config: Config, shared: Arc<Mutex<SharedState>>) -> Self {
        Self {
            gateway_url: config.gateway.url.clone().unwrap_or_default(),
            gateway_token: config.gateway.token.clone().unwrap_or_default(),
            auto_restart: config.widget.auto_restart,
            auto_start: autostart::effective_autostart(&config),
            check_interval: config.widget.check_interval_secs as f32,
            notifications: config.widget.notifications,
            notification_sound: config.widget.notification_sound,
            save_message: None,
            save_error: None,
            base_config: config,
            dark_mode_applied: false,
            shared,
        }
    }

    fn save_settings(&mut self) {
        self.save_message = None;
        self.save_error = None;

        let mut config = self.base_config.clone();
        config.gateway.url = if self.gateway_url.trim().is_empty() {
            None
        } else {
            Some(self.gateway_url.trim().to_string())
        };
        config.gateway.token = if self.gateway_token.trim().is_empty() {
            None
        } else {
            Some(self.gateway_token.trim().to_string())
        };
        config.widget.auto_restart = self.auto_restart;
        config.widget.check_interval_secs = self.check_interval as u64;
        config.widget.notifications = self.notifications;
        config.widget.notification_sound = self.notification_sound;
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
            self.dark_mode_applied = true;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(t("settings_title"));
            ui.add_space(12.0);

            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label(t("gateway_url"));
                    ui.text_edit_singleline(&mut self.gateway_url);
                    ui.end_row();

                    ui.label(t("gateway_token"));
                    ui.text_edit_singleline(&mut self.gateway_token);
                    ui.end_row();

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
