use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eframe::egui;

use crate::{
    autostart,
    config::Config,
    error::{AppError, Result},
    setup::{find_node_script, parse_node_script, ScriptDetection},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WizardStep {
    Welcome,
    DetectInstall,
    Gateway,
    Autostart,
    Complete,
}

#[derive(Default)]
struct WizardSharedState {
    saved_config: Option<Config>,
}

pub fn run_setup_wizard(initial_config: &Config) -> Result<Option<Config>> {
    let shared = Arc::new(Mutex::new(WizardSharedState::default()));
    let shared_for_app = Arc::clone(&shared);
    let app_config = initial_config.clone();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([500.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "OpenClaw Node Widget Setup",
        native_options,
        Box::new(
            move |_cc| -> std::result::Result<
                Box<dyn eframe::App>,
                Box<dyn std::error::Error + Send + Sync>,
            > {
                Ok(Box::new(SetupWizardApp::new(
                    app_config.clone(),
                    Arc::clone(&shared_for_app),
                )))
            },
        ),
    )
    .map_err(|err| AppError::Tray(format!("failed to launch setup wizard: {err}")))?;

    let saved = shared
        .lock()
        .map_err(|_| AppError::Config("wizard state lock poisoned".to_string()))?
        .saved_config
        .clone();

    Ok(saved)
}

struct SetupWizardApp {
    step: WizardStep,
    base_config: Config,
    shared: Arc<Mutex<WizardSharedState>>,
    script_path: Option<PathBuf>,
    detection: Option<ScriptDetection>,
    detect_message: String,
    detect_error: Option<String>,
    npm_available: bool,
    host: String,
    port: String,
    token: String,
    node_command: String,
    auto_start: bool,
    finish_error: Option<String>,
    dark_mode_applied: bool,
}

impl SetupWizardApp {
    fn new(base_config: Config, shared: Arc<Mutex<WizardSharedState>>) -> Self {
        let mut app = Self {
            step: WizardStep::Welcome,
            script_path: None,
            detection: None,
            detect_message: String::new(),
            detect_error: None,
            npm_available: false,
            host: String::new(),
            port: String::new(),
            token: base_config.gateway.token.clone().unwrap_or_default(),
            node_command: display_node_command(&base_config),
            auto_start: if config_exists() {
                autostart::effective_autostart(&base_config)
            } else {
                true
            },
            finish_error: None,
            dark_mode_applied: false,
            base_config,
            shared,
        };

        if app.host.is_empty() || app.port.is_empty() {
            let (host, port) =
                split_gateway_url(app.base_config.gateway.url.as_deref().unwrap_or_default());
            if app.host.is_empty() {
                app.host = host;
            }
            if app.port.is_empty() {
                app.port = port;
            }
        }

        app.refresh_detection();
        app
    }

    fn refresh_detection(&mut self) {
        self.detect_error = None;
        self.script_path = find_node_script();
        self.detection = None;

        if let Some(path) = &self.script_path {
            match parse_node_script(path) {
                Ok(parsed) => {
                    self.detection = parsed;
                    self.detect_message = format!("Found node script: {}", path.display());

                    if let Some(detected) = &self.detection {
                        if let Some(host) = &detected.host {
                            self.host = host.clone();
                        }
                        if let Some(port) = &detected.port {
                            self.port = port.clone();
                        }
                        if let Some(token) = &detected.token {
                            self.token = token.clone();
                        }
                    }

                    self.node_command = path.to_string_lossy().to_string();
                }
                Err(err) => {
                    self.detect_message = format!("Found node script: {}", path.display());
                    self.detect_error = Some(format!("Failed to parse node script: {err}"));
                }
            }

            self.npm_available = check_npm_available();
            return;
        }

        self.detect_message = "No node script found in ~/.openclaw".to_string();
        self.npm_available = check_npm_available();
    }

    fn run_install_flow(&mut self) {
        self.detect_error = None;

        if !self.npm_available {
            self.detect_error = Some("npm is not available. Install Node.js first.".to_string());
            return;
        }

        match run_command_with_timeout(
            "npm",
            &["install", "-g", "openclaw"],
            Duration::from_secs(10),
        ) {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                self.detect_error = Some(format!("npm install failed: {detail}"));
                return;
            }
            Err(err) => {
                self.detect_error = Some(format!("npm install failed: {err}"));
                return;
            }
        }

        let openclaw_cmd = if cfg!(windows) {
            "openclaw.cmd"
        } else {
            "openclaw"
        };

        match run_command_with_timeout(openclaw_cmd, &["node", "setup"], Duration::from_secs(10)) {
            Ok(output) if output.status.success() => {
                self.detect_message = "OpenClaw node setup completed.".to_string();
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                self.detect_error = Some(format!("openclaw node setup failed: {detail}"));
                return;
            }
            Err(err) => {
                self.detect_error = Some(format!("openclaw node setup failed: {err}"));
                return;
            }
        }

        self.refresh_detection();
    }

    fn finish_setup(&mut self) {
        self.finish_error = None;

        let host = strip_ws_scheme(self.host.trim());
        let port = self.port.trim();
        if host.is_empty() {
            self.finish_error = Some("Gateway host is required.".to_string());
            return;
        }
        if port.is_empty() {
            self.finish_error = Some("Gateway port is required.".to_string());
            return;
        }

        let node_command = self.node_command.trim();
        if node_command.is_empty() {
            self.finish_error = Some("Node command is required.".to_string());
            return;
        }

        let mut config = self.base_config.clone();
        config.gateway.url = Some(format!("ws://{host}:{port}"));
        config.gateway.token = if self.token.trim().is_empty() {
            None
        } else {
            Some(self.token.trim().to_string())
        };
        config.startup.auto_start = self.auto_start;
        apply_node_command(&mut config, node_command);

        if let Err(err) = autostart::set_autostart(self.auto_start) {
            self.finish_error = Some(format!("Failed to configure autostart: {err}"));
            return;
        }

        if let Err(err) = config.save() {
            self.finish_error = Some(format!("Failed to save config: {err}"));
            return;
        }

        match self.shared.lock() {
            Ok(mut state) => {
                state.saved_config = Some(config.clone());
            }
            Err(_) => {
                self.finish_error = Some("Failed to update wizard state.".to_string());
                return;
            }
        }

        self.base_config = config;
        self.step = WizardStep::Complete;
    }

    fn nav_buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if self.step != WizardStep::Welcome && self.step != WizardStep::Complete {
                if ui.button("Back").clicked() {
                    self.step = match self.step {
                        WizardStep::DetectInstall => WizardStep::Welcome,
                        WizardStep::Gateway => WizardStep::DetectInstall,
                        WizardStep::Autostart => WizardStep::Gateway,
                        _ => self.step,
                    };
                }
            }

            if self.step != WizardStep::Complete {
                if ui.button("Cancel").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }

            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match self.step {
                    WizardStep::Welcome => {
                        if ui.button("Next").clicked() {
                            self.step = WizardStep::DetectInstall;
                        }
                    }
                    WizardStep::DetectInstall => {
                        if ui.button("Next").clicked() {
                            self.step = WizardStep::Gateway;
                        }
                    }
                    WizardStep::Gateway => {
                        if ui.button("Next").clicked() {
                            self.step = WizardStep::Autostart;
                        }
                    }
                    WizardStep::Autostart => {
                        if ui.button("Finish").clicked() {
                            self.finish_setup();
                        }
                    }
                    WizardStep::Complete => {
                        if ui.button("Done").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                },
            );
        });
    }
}

impl eframe::App for SetupWizardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.dark_mode_applied {
            ctx.set_visuals(egui::Visuals::dark());
            self.dark_mode_applied = true;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("OpenClaw Node Widget Setup");
            ui.add_space(12.0);

            match self.step {
                WizardStep::Welcome => {
                    ui.heading("Welcome");
                    ui.label("Welcome to OpenClaw Node Widget.");
                    ui.label("This wizard will help you set up your OpenClaw Node connection.");
                }
                WizardStep::DetectInstall => {
                    ui.heading("Detect / Install Node");
                    ui.label(&self.detect_message);

                    if let Some(path) = &self.script_path {
                        ui.monospace(path.display().to_string());
                    }

                    if let Some(detected) = &self.detection {
                        if let Some(host) = &detected.host {
                            ui.label(format!("Detected host: {host}"));
                        }
                        if let Some(port) = &detected.port {
                            ui.label(format!("Detected port: {port}"));
                        }
                        if let Some(token) = &detected.token {
                            ui.label(format!("Detected token: {token}"));
                        }
                    }

                    if self.script_path.is_none() {
                        if self.npm_available {
                            ui.label("npm is available. You can install OpenClaw now.");
                            if ui.button("Install OpenClaw Node").clicked() {
                                self.run_install_flow();
                            }
                        } else {
                            ui.label("npm not found. Please install Node.js first.");
                            if ui.button("Open nodejs.org").clicked() {
                                let _ = open::that("https://nodejs.org");
                            }
                        }
                    }

                    if ui.button("Re-detect").clicked() {
                        self.refresh_detection();
                    }

                    if let Some(err) = &self.detect_error {
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                    }
                }
                WizardStep::Gateway => {
                    ui.heading("Gateway Configuration");
                    ui.label("Gateway Host");
                    ui.text_edit_singleline(&mut self.host);
                    ui.label("Gateway Port");
                    ui.text_edit_singleline(&mut self.port);
                    ui.label("Gateway Token (optional)");
                    ui.text_edit_singleline(&mut self.token);
                    ui.label("Node command");
                    ui.text_edit_singleline(&mut self.node_command);
                }
                WizardStep::Autostart => {
                    ui.heading("Autostart");
                    ui.checkbox(&mut self.auto_start, "Start widget on login");

                    if let Some(err) = &self.finish_error {
                        ui.add_space(8.0);
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                    }
                }
                WizardStep::Complete => {
                    ui.heading("Complete");
                    ui.label("Setup complete! The widget will now start monitoring your node.");
                }
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.separator();
                self.nav_buttons(ui, ctx);
            });
        });
    }
}

fn check_npm_available() -> bool {
    run_command_with_timeout("npm", &["--version"], Duration::from_secs(10))
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> std::result::Result<Output, String> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    wait_with_timeout(child, timeout)
}

fn wait_with_timeout(mut child: Child, timeout: Duration) -> std::result::Result<Output, String> {
    let start = Instant::now();

    loop {
        if let Some(_status) = child.try_wait().map_err(|e| e.to_string())? {
            return child.wait_with_output().map_err(|e| e.to_string());
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            return Err(format!("timed out after {}s", timeout.as_secs()));
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn split_gateway_url(url: &str) -> (String, String) {
    let trimmed = strip_ws_scheme(url.trim());
    if let Some((host, port)) = trimmed.rsplit_once(':') {
        if !host.is_empty() && !port.is_empty() {
            return (host.to_string(), port.to_string());
        }
    }
    (String::new(), String::new())
}

fn strip_ws_scheme(input: &str) -> &str {
    input
        .strip_prefix("ws://")
        .or_else(|| input.strip_prefix("wss://"))
        .unwrap_or(input)
}

fn display_node_command(config: &Config) -> String {
    #[cfg(windows)]
    {
        if config.node.command.eq_ignore_ascii_case("cmd.exe")
            && config.node.args.len() >= 2
            && config.node.args[0].eq_ignore_ascii_case("/c")
        {
            return config.node.args[1].clone();
        }
    }

    if config.node.args.is_empty() {
        return config.node.command.clone();
    }

    format!("{} {}", config.node.command, config.node.args.join(" "))
}

fn apply_node_command(config: &mut Config, input: &str) {
    let trimmed = input.trim();

    #[cfg(windows)]
    {
        if looks_like_script_path(trimmed) {
            config.node.command = "cmd.exe".to_string();
            config.node.args = vec!["/c".to_string(), trimmed.to_string()];
            return;
        }
    }

    #[cfg(not(windows))]
    {
        if looks_like_script_path(trimmed) {
            config.node.command = trimmed.to_string();
            config.node.args.clear();
            return;
        }
    }

    config.node.command = trimmed.to_string();
    config.node.args.clear();
}

fn looks_like_script_path(value: &str) -> bool {
    let path = Path::new(value);
    if path.is_absolute() {
        return true;
    }

    value.ends_with(".cmd") || value.ends_with(".sh") || value.starts_with("./")
}

fn config_exists() -> bool {
    crate::config::config_path()
        .map(|path| path.exists())
        .unwrap_or(false)
}
