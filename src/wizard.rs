use std::{
    net::TcpStream,
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
    i18n::t,
    setup::{find_node_script, parse_node_script, ScriptDetection},
    tailscale::{TailscalePeer, TailscaleStatus},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WizardStep {
    Welcome,
    DetectInstall,
    Tailscale,
    Gateway,
    Pairing,
    Autostart,
    Complete,
}

/// Whether we should show the "Install to system?" step in the wizard.
/// Only on Windows when not already running from the install directory.
fn should_offer_install() -> bool {
    #[cfg(windows)]
    {
        !crate::install::is_running_from_install_dir()
    }
    #[cfg(not(windows))]
    {
        false
    }
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
        viewport: egui::ViewportBuilder::default().with_inner_size([500.0, 440.0]),
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

/// Result of a Gateway TCP connection test.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionTestResult {
    Untested,
    Success,
    Failed(String),
}

/// Pairing status from the Gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PairingStatus {
    Unknown,
    Checking,
    AlreadyPaired,
    Waiting,
    #[allow(dead_code)]
    Approved,
    TimedOut,
    Failed(String),
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
    nodejs_available: bool,
    installing_nodejs: bool,
    host: String,
    port: String,
    token: String,
    node_command: String,
    auto_start: bool,
    install_to_system: bool,
    finish_error: Option<String>,
    dark_mode_applied: bool,
    tailscale_peers: Vec<TailscalePeer>,
    tailscale_status: TailscaleStatus,
    selected_peer_index: Option<usize>,
    connection_test: ConnectionTestResult,
    pairing_status: PairingStatus,
    pairing_start: Option<Instant>,
}

impl SetupWizardApp {
    fn new(base_config: Config, shared: Arc<Mutex<WizardSharedState>>) -> Self {
        // Pre-fill from defaults if present
        let default_host = base_config
            .defaults
            .gateway_host
            .clone()
            .unwrap_or_default();
        let default_port = base_config
            .defaults
            .gateway_port
            .clone()
            .unwrap_or_default();
        let default_token = base_config
            .defaults
            .gateway_token
            .clone()
            .or_else(|| base_config.gateway.token.clone())
            .unwrap_or_default();

        let mut app = Self {
            step: WizardStep::Welcome,
            script_path: None,
            detection: None,
            detect_message: String::new(),
            detect_error: None,
            npm_available: false,
            nodejs_available: false,
            installing_nodejs: false,
            host: default_host,
            port: default_port,
            token: default_token,
            node_command: display_node_command(&base_config),
            auto_start: if config_exists() {
                autostart::effective_autostart(&base_config)
            } else {
                true
            },
            install_to_system: should_offer_install(),
            finish_error: None,
            dark_mode_applied: false,
            tailscale_peers: Vec::new(),
            tailscale_status: TailscaleStatus::NotInstalled,
            selected_peer_index: None,
            connection_test: ConnectionTestResult::Untested,
            pairing_status: PairingStatus::Unknown,
            pairing_start: None,
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

    fn detect_tailscale(&mut self) {
        self.tailscale_status = crate::tailscale::check_status();
        let peers = crate::tailscale::detect_peers();
        self.tailscale_peers = peers;
        self.selected_peer_index = None;
    }

    fn refresh_detection(&mut self) {
        self.detect_error = None;
        self.script_path = find_node_script();
        self.detection = None;
        self.detect_tailscale();
        self.nodejs_available = crate::config::detect_nodejs();

        if let Some(path) = &self.script_path {
            match parse_node_script(path) {
                Ok(parsed) => {
                    self.detection = parsed;
                    self.detect_message =
                        format!("{}{}", t("found_node_script"), path.display());

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
                    self.detect_message =
                        format!("{}{}", t("found_node_script"), path.display());
                    self.detect_error = Some(format!("Failed to parse node script: {err}"));
                }
            }

            self.npm_available = check_npm_available();
            return;
        }

        self.detect_message = t("no_node_script").to_string();
        self.npm_available = check_npm_available();
    }

    fn run_install_flow(&mut self) {
        self.detect_error = None;

        if !self.npm_available {
            self.detect_error = Some(t("npm_not_found").to_string());
            return;
        }

        // Use pre-configured defaults for openclaw node setup if available
        let mut setup_args: Vec<&str> = vec!["node", "setup"];
        let host_flag;
        let port_flag;

        if let Some(h) = &self.base_config.defaults.gateway_host {
            host_flag = h.clone();
            setup_args.push("--host");
            setup_args.push(&host_flag);
        }
        if let Some(p) = &self.base_config.defaults.gateway_port {
            port_flag = p.clone();
            setup_args.push("--port");
            setup_args.push(&port_flag);
        }

        match run_command_with_timeout(
            "npm",
            &["install", "-g", "openclaw"],
            Duration::from_secs(120),
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

        let setup_arg_refs: Vec<&str> = setup_args.to_vec();
        match run_command_with_timeout(openclaw_cmd, &setup_arg_refs, Duration::from_secs(30)) {
            Ok(output) if output.status.success() => {
                self.detect_message = t("setup_completed").to_string();
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

    /// Platform-specific Node.js install flow.
    fn install_nodejs(&mut self) {
        self.installing_nodejs = true;
        self.detect_error = None;

        #[cfg(windows)]
        {
            // Windows: attempt silent MSI install
            self.detect_error = Some(t("nodejs_install_win").to_string());
            // Download LTS info
            match run_command_with_timeout(
                "powershell",
                &[
                    "-Command",
                    "(Invoke-WebRequest -Uri 'https://nodejs.org/dist/index.json' -UseBasicParsing).Content | ConvertFrom-Json | Where-Object { $_.lts -ne $false } | Select-Object -First 1 -ExpandProperty version",
                ],
                Duration::from_secs(30),
            ) {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !version.is_empty() {
                        let msi_url = format!(
                            "https://nodejs.org/dist/{version}/node-{version}-x64.msi"
                        );
                        let temp_dir = std::env::temp_dir();
                        let msi_path = temp_dir.join(format!("node-{version}-x64.msi"));
                        let msi_path_str = msi_path.to_string_lossy().to_string();

                        // Download MSI
                        match run_command_with_timeout(
                            "powershell",
                            &[
                                "-Command",
                                &format!(
                                    "Invoke-WebRequest -Uri '{msi_url}' -OutFile '{msi_path_str}'"
                                ),
                            ],
                            Duration::from_secs(120),
                        ) {
                            Ok(dl) if dl.status.success() => {
                                // Run MSI silently
                                match run_command_with_timeout(
                                    "msiexec",
                                    &["/i", &msi_path_str, "/quiet", "/norestart"],
                                    Duration::from_secs(120),
                                ) {
                                    Ok(inst) if inst.status.success() => {
                                        self.detect_error = None;
                                        self.nodejs_available = true;
                                        self.npm_available = check_npm_available();
                                    }
                                    Ok(inst) => {
                                        let stderr =
                                            String::from_utf8_lossy(&inst.stderr).trim().to_string();
                                        self.detect_error =
                                            Some(format!("{}: {stderr}", t("install_failed")));
                                    }
                                    Err(e) => {
                                        self.detect_error =
                                            Some(format!("{}: {e}", t("install_failed")));
                                    }
                                }
                                // Cleanup temp MSI
                                let _ = std::fs::remove_file(&msi_path);
                            }
                            Ok(_) => {
                                self.detect_error =
                                    Some(format!("{}: download failed", t("install_failed")));
                            }
                            Err(e) => {
                                self.detect_error =
                                    Some(format!("{}: {e}", t("install_failed")));
                            }
                        }
                    } else {
                        self.detect_error =
                            Some(format!("{}: could not determine LTS version", t("install_failed")));
                    }
                }
                Ok(_) | Err(_) => {
                    self.detect_error =
                        Some(format!("{}: could not fetch Node.js version info", t("install_failed")));
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            // macOS: open nodejs.org in browser
            let _ = open::that("https://nodejs.org");
            self.detect_error = Some(t("nodejs_install_mac").to_string());
        }

        #[cfg(target_os = "linux")]
        {
            // Linux: show install command hint
            self.detect_error = Some(t("nodejs_install_linux").to_string());
        }

        self.installing_nodejs = false;
    }

    /// Test TCP connection to the configured Gateway host:port.
    fn test_connection(&mut self) {
        let host = strip_ws_scheme(self.host.trim());
        let port = self.port.trim();

        if host.is_empty() || port.is_empty() {
            self.connection_test =
                ConnectionTestResult::Failed("Host and port are required".to_string());
            return;
        }

        let addr = format!("{host}:{port}");
        match TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| {
                // Fallback: try to resolve
                use std::net::ToSocketAddrs;
                addr.to_socket_addrs()
                    .ok()
                    .and_then(|mut a| a.next())
                    .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap())
            }),
            Duration::from_secs(5),
        ) {
            Ok(_) => {
                self.connection_test = ConnectionTestResult::Success;
            }
            Err(e) => {
                self.connection_test = ConnectionTestResult::Failed(e.to_string());
            }
        }
    }

    /// Check pairing status via HTTP GET to the Gateway API.
    fn check_pairing(&mut self) {
        let host = strip_ws_scheme(self.host.trim());
        let port = self.port.trim();

        if host.is_empty() || port.is_empty() {
            self.pairing_status = PairingStatus::Failed("Gateway not configured".to_string());
            return;
        }

        self.pairing_status = PairingStatus::Checking;
        let url = format!("http://{host}:{port}/api/nodes");

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build();

        let client = match client {
            Ok(c) => c,
            Err(e) => {
                self.pairing_status = PairingStatus::Failed(e.to_string());
                return;
            }
        };

        let mut request = client.get(&url);
        if !self.token.trim().is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.token.trim()));
        }

        match request.send() {
            Ok(resp) if resp.status().is_success() => {
                // If we can reach /api/nodes, consider it paired
                self.pairing_status = PairingStatus::AlreadyPaired;
            }
            Ok(resp) if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 => {
                // Not yet authorized — waiting for approval
                self.pairing_status = PairingStatus::Waiting;
                if self.pairing_start.is_none() {
                    self.pairing_start = Some(Instant::now());
                }
            }
            Ok(resp) => {
                self.pairing_status =
                    PairingStatus::Failed(format!("HTTP {}", resp.status()));
            }
            Err(e) => {
                self.pairing_status = PairingStatus::Failed(e.to_string());
            }
        }
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

        // Perform system install if requested (Windows only)
        #[cfg(windows)]
        if self.install_to_system {
            if let Err(err) = crate::install::perform_install() {
                self.finish_error = Some(format!("Install failed: {err}"));
                return;
            }
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
            if self.step != WizardStep::Welcome
                && self.step != WizardStep::Complete
                && ui.button(t("back")).clicked()
            {
                self.step = match self.step {
                    WizardStep::DetectInstall => WizardStep::Welcome,
                    WizardStep::Tailscale => WizardStep::DetectInstall,
                    WizardStep::Gateway => WizardStep::Tailscale,
                    WizardStep::Pairing => WizardStep::Gateway,
                    WizardStep::Autostart => WizardStep::Pairing,
                    _ => self.step,
                };
            }

            if self.step != WizardStep::Complete
                && ui.button(t("cancel")).clicked()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }

            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match self.step {
                    WizardStep::Welcome => {
                        if ui.button(t("next")).clicked() {
                            self.step = WizardStep::DetectInstall;
                        }
                    }
                    WizardStep::DetectInstall => {
                        if ui.button(t("next")).clicked() {
                            self.step = WizardStep::Tailscale;
                        }
                    }
                    WizardStep::Tailscale => {
                        if ui.button(t("next")).clicked() {
                            self.step = WizardStep::Gateway;
                        }
                    }
                    WizardStep::Gateway => {
                        if ui.button(t("next")).clicked() {
                            self.pairing_status = PairingStatus::Unknown;
                            self.pairing_start = None;
                            self.step = WizardStep::Pairing;
                        }
                    }
                    WizardStep::Pairing => {
                        if ui.button(t("next")).clicked() {
                            self.step = WizardStep::Autostart;
                        }
                    }
                    WizardStep::Autostart => {
                        if ui.button(t("finish")).clicked() {
                            self.finish_setup();
                        }
                    }
                    WizardStep::Complete => {
                        if ui.button(t("done")).clicked() {
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
            crate::i18n::setup_cjk_fonts(ctx);
            self.dark_mode_applied = true;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(t("wizard_title"));
            ui.add_space(12.0);

            match self.step {
                WizardStep::Welcome => {
                    ui.heading(t("welcome"));
                    ui.label(t("welcome_msg"));
                    ui.label(t("welcome_desc"));
                }
                WizardStep::DetectInstall => {
                    ui.heading(t("detect_install"));
                    ui.label(&self.detect_message);

                    if let Some(path) = &self.script_path {
                        ui.monospace(path.display().to_string());
                    }

                    if let Some(detected) = &self.detection {
                        if let Some(host) = &detected.host {
                            ui.label(format!("{}{host}", t("detected_host")));
                        }
                        if let Some(port) = &detected.port {
                            ui.label(format!("{}{port}", t("detected_port")));
                        }
                        if let Some(token) = &detected.token {
                            ui.label(format!("{}{token}", t("detected_token")));
                        }
                    }

                    if self.script_path.is_none() {
                        if self.npm_available {
                            ui.label(t("npm_available"));
                            if ui.button(t("install_openclaw")).clicked() {
                                self.run_install_flow();
                            }
                        } else if self.nodejs_available {
                            // Node.js exists but npm check failed — still offer install
                            ui.label(t("npm_not_found"));
                            if ui.button(t("open_nodejs")).clicked() {
                                let _ = open::that("https://nodejs.org");
                            }
                        } else {
                            // No Node.js at all
                            ui.label(t("nodejs_required"));
                            ui.add_space(4.0);

                            if cfg!(windows) {
                                if ui.button(t("install_nodejs")).clicked() {
                                    self.install_nodejs();
                                }
                            } else if cfg!(target_os = "macos") {
                                if ui.button(t("install_nodejs")).clicked() {
                                    self.install_nodejs();
                                }
                                ui.label(t("nodejs_install_mac"));
                            } else {
                                // Linux
                                ui.label(t("nodejs_install_linux"));
                            }
                        }
                    }

                    if ui.button(t("redetect")).clicked() {
                        self.refresh_detection();
                    }

                    if let Some(err) = &self.detect_error {
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                    }
                }
                WizardStep::Tailscale => {
                    ui.heading(t("tailscale_step_title"));

                    match self.tailscale_status {
                        TailscaleStatus::NotInstalled => {
                            ui.add_space(8.0);
                            ui.label(t("tailscale_optional_desc"));
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                if ui.button(t("tailscale_install_btn")).clicked() {
                                    let _ =
                                        open::that("https://tailscale.com/download");
                                }
                                if ui.button(t("tailscale_skip")).clicked() {
                                    self.step = WizardStep::Gateway;
                                }
                            });
                        }
                        TailscaleStatus::Disconnected => {
                            ui.add_space(8.0);
                            ui.label(t("tailscale_disconnected_msg"));
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                if ui.button(t("tailscale_open_btn")).clicked() {
                                    // Try to open Tailscale app
                                    #[cfg(target_os = "macos")]
                                    {
                                        let _ = open::that(
                                            "file:///Applications/Tailscale.app",
                                        );
                                    }
                                    #[cfg(target_os = "linux")]
                                    {
                                        let _ = Command::new("tailscale")
                                            .arg("up")
                                            .spawn();
                                    }
                                    #[cfg(windows)]
                                    {
                                        let _ = open::that(
                                            "tailscale:",
                                        );
                                    }
                                }
                                if ui.button(t("tailscale_skip")).clicked() {
                                    self.step = WizardStep::Gateway;
                                }
                            });

                            if ui.button(t("redetect")).clicked() {
                                self.detect_tailscale();
                            }
                        }
                        TailscaleStatus::Connected => {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(t("tailscale_connected_label"))
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                            ui.add_space(8.0);
                            ui.label(t("tailscale_select_gateway"));

                            let peers = self.tailscale_peers.clone();
                            let mut selected = self.selected_peer_index;

                            for (i, peer) in peers.iter().enumerate() {
                                let label =
                                    format!("{} ({})", peer.hostname, peer.ip);
                                let is_selected = selected == Some(i);
                                if ui
                                    .selectable_label(is_selected, &label)
                                    .clicked()
                                {
                                    selected = Some(i);
                                    self.host = peer.ip.clone();
                                    self.port = "18789".to_string();
                                }
                            }

                            if ui
                                .selectable_label(
                                    selected.is_none(),
                                    t("tailscale_manual_entry"),
                                )
                                .clicked()
                            {
                                selected = None;
                            }
                            self.selected_peer_index = selected;
                        }
                    }
                }
                WizardStep::Gateway => {
                    ui.heading(t("gateway_config"));

                    ui.add_space(4.0);
                    ui.label(t("gateway_host"));
                    ui.text_edit_singleline(&mut self.host);
                    ui.label(t("gateway_port"));
                    ui.text_edit_singleline(&mut self.port);
                    ui.label(t("gateway_token_optional"));
                    ui.text_edit_singleline(&mut self.token);
                    ui.label(t("node_command"));
                    ui.text_edit_singleline(&mut self.node_command);

                    ui.add_space(8.0);

                    // Connection test button
                    if ui.button(t("test_connection")).clicked() {
                        self.test_connection();
                    }

                    match &self.connection_test {
                        ConnectionTestResult::Untested => {}
                        ConnectionTestResult::Success => {
                            ui.colored_label(
                                egui::Color32::from_rgb(100, 200, 100),
                                format!("\u{2705} {}", t("connection_success")),
                            );
                        }
                        ConnectionTestResult::Failed(msg) => {
                            ui.colored_label(
                                egui::Color32::from_rgb(220, 80, 80),
                                format!(
                                    "\u{274C} {} — {}",
                                    t("connection_failed"),
                                    t("connection_failed_hint")
                                ),
                            );
                            ui.label(
                                egui::RichText::new(msg)
                                    .color(egui::Color32::from_rgb(150, 150, 150))
                                    .small(),
                            );
                        }
                    }
                }
                WizardStep::Pairing => {
                    ui.heading(t("pairing_title"));
                    ui.add_space(8.0);

                    // Auto-check pairing on first render of this step
                    if self.pairing_status == PairingStatus::Unknown {
                        self.check_pairing();
                    }

                    match &self.pairing_status {
                        PairingStatus::Unknown | PairingStatus::Checking => {
                            ui.label(t("pairing_checking"));
                            ui.spinner();
                        }
                        PairingStatus::AlreadyPaired => {
                            ui.colored_label(
                                egui::Color32::from_rgb(100, 200, 100),
                                format!("\u{2705} {}", t("pairing_already_paired")),
                            );
                        }
                        PairingStatus::Waiting => {
                            ui.label(t("pairing_waiting"));
                            ui.spinner();

                            // Check for timeout (120s)
                            if let Some(start) = self.pairing_start {
                                if start.elapsed() > Duration::from_secs(120) {
                                    self.pairing_status = PairingStatus::TimedOut;
                                }
                            }

                            // Auto-poll every ~3s by requesting repaint
                            ctx.request_repaint_after(Duration::from_secs(3));
                        }
                        PairingStatus::Approved => {
                            ui.colored_label(
                                egui::Color32::from_rgb(100, 200, 100),
                                format!("\u{2705} {}", t("pairing_approved")),
                            );
                        }
                        PairingStatus::TimedOut => {
                            ui.colored_label(
                                egui::Color32::from_rgb(220, 180, 50),
                                t("pairing_timeout"),
                            );
                            if ui.button(t("retry")).clicked() {
                                self.pairing_status = PairingStatus::Unknown;
                                self.pairing_start = None;
                            }
                        }
                        PairingStatus::Failed(msg) => {
                            ui.colored_label(
                                egui::Color32::from_rgb(220, 80, 80),
                                msg,
                            );
                            if ui.button(t("retry")).clicked() {
                                self.pairing_status = PairingStatus::Unknown;
                                self.pairing_start = None;
                            }
                        }
                    }
                }
                WizardStep::Autostart => {
                    ui.heading(t("autostart"));
                    ui.checkbox(&mut self.auto_start, t("start_on_login"));

                    if should_offer_install() {
                        ui.add_space(12.0);
                        ui.checkbox(
                            &mut self.install_to_system,
                            "Install to system (Recommended)",
                        );
                        ui.label(
                            "Installs to a standard location, adds Start Menu shortcut.",
                        );
                    }

                    if let Some(err) = &self.finish_error {
                        ui.add_space(8.0);
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                    }
                }
                WizardStep::Complete => {
                    ui.heading(t("complete"));
                    ui.label(t("complete_msg"));
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
