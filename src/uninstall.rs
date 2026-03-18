use std::{
    fs,
    sync::{Arc, Mutex},
};

use eframe::egui;

use crate::{
    autostart,
    config,
    error::{AppError, Result},
    i18n::t,
};

pub fn confirm_uninstall() -> Result<bool> {
    let confirmed = Arc::new(Mutex::new(false));
    let confirmed_clone = Arc::clone(&confirmed);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([400.0, 180.0]),
        ..Default::default()
    };

    eframe::run_native(
        t("confirm_uninstall"),
        options,
        Box::new(
            move |_cc| -> std::result::Result<
                Box<dyn eframe::App>,
                Box<dyn std::error::Error + Send + Sync>,
            > {
                Ok(Box::new(UninstallDialog {
                    confirmed: confirmed_clone,
                    dark_mode_applied: false,
                }))
            },
        ),
    )
    .map_err(|e| AppError::Tray(format!("uninstall dialog failed: {e}")))?;

    let result = *confirmed.lock().unwrap_or_else(|e| e.into_inner());
    Ok(result)
}

pub fn perform_uninstall() -> Result<()> {
    let _ = autostart::set_autostart(false);

    if let Ok(dir) = config::app_dir() {
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
    }

    Ok(())
}

struct UninstallDialog {
    confirmed: Arc<Mutex<bool>>,
    dark_mode_applied: bool,
}

impl eframe::App for UninstallDialog {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.dark_mode_applied {
            ctx.set_visuals(egui::Visuals::dark());
            self.dark_mode_applied = true;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(t("confirm_uninstall"));
            ui.add_space(12.0);
            ui.label(t("uninstall_msg"));
            ui.add_space(16.0);

            ui.horizontal(|ui| {
                if ui.button(t("yes_uninstall")).clicked() {
                    if let Ok(mut c) = self.confirmed.lock() {
                        *c = true;
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button(t("cancel")).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}
