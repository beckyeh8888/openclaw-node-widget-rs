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
    tracing::info!("performing uninstall...");

    // Disable autostart (all platforms)
    let _ = autostart::set_autostart(false);

    // Remove config directory (contains config.toml, device keys, etc.)
    if let Ok(dir) = config::app_dir() {
        if dir.exists() {
            tracing::info!("removing config dir: {}", dir.display());
            let _ = fs::remove_dir_all(&dir);
        }
    }

    // Platform-specific cleanup
    #[cfg(windows)]
    perform_uninstall_windows();

    #[cfg(target_os = "macos")]
    perform_uninstall_macos();

    #[cfg(target_os = "linux")]
    perform_uninstall_linux();

    Ok(())
}

#[cfg(windows)]
fn perform_uninstall_windows() {
    // Remove installed exe directory
    crate::install::remove_install_dir();

    // Remove autostart registry entry (belt-and-suspenders, set_autostart should have done this)
    let _ = remove_windows_autostart_registry();

    // Remove Start Menu shortcut
    crate::install::remove_start_menu_shortcut();
}

#[cfg(windows)]
fn remove_windows_autostart_registry() -> Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    if let Ok(run_key) = hkcu.open_subkey_with_flags(path, winreg::enums::KEY_WRITE) {
        let _ = run_key.delete_value("OpenClawNodeWidget");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn perform_uninstall_macos() {
    // Remove launchd plist (belt-and-suspenders)
    let plist = dirs::home_dir()
        .unwrap_or_default()
        .join("Library")
        .join("LaunchAgents")
        .join("ai.openclaw.node-widget.plist");
    if plist.exists() {
        tracing::info!("removing launchd plist: {}", plist.display());
        let _ = fs::remove_file(plist);
    }
}

#[cfg(target_os = "linux")]
fn perform_uninstall_linux() {
    // Remove autostart .desktop file (belt-and-suspenders)
    let desktop = dirs::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("autostart")
        .join("openclaw-node-widget.desktop");
    if desktop.exists() {
        tracing::info!("removing desktop file: {}", desktop.display());
        let _ = fs::remove_file(desktop);
    }
}

struct UninstallDialog {
    confirmed: Arc<Mutex<bool>>,
    dark_mode_applied: bool,
}

impl eframe::App for UninstallDialog {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.dark_mode_applied {
            ctx.set_visuals(egui::Visuals::dark());
            crate::i18n::setup_cjk_fonts(ctx);
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
