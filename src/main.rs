// Hide console window on Windows when not running from terminal
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod chat;
mod config;
pub mod dashboard;
mod error;
mod gateway;
mod history;
mod i18n;
mod install;
mod markdown;
mod lock;
mod monitor;
pub mod plugin;
mod process;
mod settings;
mod setup;
mod tailscale;
mod tray;
mod uninstall;
mod update;
mod voice;
mod wizard;

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use std::sync::{Arc, Mutex};

use crate::{
    config::{config_path, Config},
    history::ChatHistory,
    lock::{try_acquire_lock, AcquireResult},
    monitor::{spawn_monitor, MonitorCommand},
    plugin::registry::PluginRegistry,
    tray::{TrayCommand, TrayState},
};

/// Rate-limiter for plugin notifications.
struct NotificationLimiter {
    last_notify: std::collections::HashMap<String, std::time::Instant>,
    cooldown: std::time::Duration,
}

impl NotificationLimiter {
    fn new(cooldown_secs: u64) -> Self {
        Self {
            last_notify: std::collections::HashMap::new(),
            cooldown: std::time::Duration::from_secs(cooldown_secs),
        }
    }

    /// Returns true if a notification is allowed for this plugin.
    fn allow(&mut self, plugin_id: &str) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.last_notify.get(plugin_id) {
            if now.duration_since(*last) < self.cooldown {
                return false;
            }
        }
        self.last_notify.insert(plugin_id.to_string(), now);
        true
    }
}

#[derive(Debug, Parser)]
#[command(name = "openclaw-node-widget")]
#[command(about = "OpenClaw Node tray widget")]
struct Cli {
    #[arg(long)]
    gateway: Option<String>,
    #[arg(long)]
    token: Option<String>,
    /// Install the widget to the system install path (Windows)
    #[arg(long)]
    install: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Clone, Subcommand)]
enum Commands {
    Run,
    Setup,
    Daemon,
    Status,
    Stop,
    Restart,
    Config,
}

#[tokio::main]
async fn main() {
    match run().await {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            error!("{err}");
            std::process::exit(1);
        }
    }
}

async fn run() -> error::Result<i32> {
    let cli = Cli::parse();

    // Handle --install flag (Windows: copy exe, set autostart, create shortcut)
    #[cfg(windows)]
    if cli.install {
        match install::perform_install() {
            Ok(()) => {
                tray::send_notification_public("OpenClaw Node Widget installed successfully!");
                // Launch from install path and exit
                install::launch_installed_and_exit();
            }
            Err(e) => {
                error!("install failed: {e}");
                return Ok(1);
            }
        }
    }
    #[cfg(not(windows))]
    if cli.install {
        println!("--install is only supported on Windows. On other platforms, just run the binary directly.");
        return Ok(0);
    }

    let command = cli.command.unwrap_or(Commands::Run);
    let config_exists = config_path()?.exists();

    let mut config = Config::load()?;

    // CLI overrides apply to the first connection (backward compat)
    if let Some(url) = cli.gateway {
        if config.connections.is_empty() {
            config.gateway.url = Some(url);
        } else if let Some(first) = config.connections.first_mut() {
            first.gateway_url = url;
        }
    }
    if let Some(token) = cli.token {
        if config.connections.is_empty() {
            config.gateway.token = Some(token);
        } else if let Some(first) = config.connections.first_mut() {
            first.gateway_token = Some(token);
        }
    }

    match command {
        Commands::Run => {
            let _lock_guard = match try_acquire_lock()? {
                AcquireResult::Acquired(guard) => guard,
                AcquireResult::AlreadyRunning(pid) => {
                    println!("Widget is already running (PID {pid})");
                    return Ok(1);
                }
            };

            if !config_exists {
                match wizard::run_setup_wizard(&config)? {
                    Some(saved_config) => {
                        config = saved_config;
                        // First-run tutorial: show a helpful notification
                        tray::send_notification_public(
                            "Widget is running! Right-click the tray icon to see options.",
                        );
                    }
                    None => return Ok(0),
                }
            }
            init_tracing(&config);
            let warnings = config.validate();
            for w in &warnings {
                tracing::warn!("config validation: {w}");
            }
            if let Some(first) = warnings.first() {
                tray::send_notification_public(&format!("Config warning: {first}"));
            }
            run_with_tray(config).await?;
            Ok(0)
        }
        Commands::Daemon => {
            let _lock_guard = match try_acquire_lock()? {
                AcquireResult::Acquired(guard) => guard,
                AcquireResult::AlreadyRunning(pid) => {
                    println!("Widget is already running (PID {pid})");
                    return Ok(1);
                }
            };

            init_tracing(&config);
            run_daemon(config).await?;
            Ok(0)
        }
        Commands::Status => run_status(),
        Commands::Stop => {
            process::stop_node()?;
            println!("Node stopped");
            Ok(0)
        }
        Commands::Restart => {
            let _ = process::stop_node();
            process::start_node(&config)?;
            println!("Node restarted");
            Ok(0)
        }
        Commands::Config => {
            println!("{}", toml::to_string_pretty(&config).unwrap_or_default());
            Ok(0)
        }
        Commands::Setup => {
            if wizard::run_setup_wizard(&config)?.is_some() {
                println!("Setup complete.");
            } else {
                println!("Setup canceled.");
            }
            Ok(0)
        }
    }
}

fn init_tracing(config: &Config) {
    let level = config.log.level.clone();
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn run_with_tray(mut config: Config) -> error::Result<()> {
    i18n::init_with_config(&config.widget.language);
    update::spawn_periodic_check();

    let (tray_cmd_tx, mut tray_cmd_rx) = mpsc::unbounded_channel();
    let tray_cmd_tx2 = tray_cmd_tx.clone();
    let tray_cmd_tx_hotkey = tray_cmd_tx.clone();
    let (monitor_cmd_tx, monitor_cmd_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();
    let (gateway_event_tx, mut gateway_event_rx) = mpsc::unbounded_channel();
    let (gateway_monitor_tx, gateway_monitor_rx) = mpsc::unbounded_channel();

    let chat_state = Arc::new(Mutex::new(chat::ChatState::new()));
    let mut chat_history = ChatHistory::load();
    let mut notification_limiter = NotificationLimiter::new(30);
    let mut last_history_save = std::time::Instant::now();
    let start_time = std::time::Instant::now();
    let mut latency_tracker = dashboard::LatencyTracker::new();
    let mut last_dashboard_push = std::time::Instant::now();
    let mut health_tracker = dashboard::HealthTracker::new();
    let mut last_health_check = std::time::Instant::now();

    // ── Build PluginRegistry from config ────────────────────────────
    let mut plugin_registry = PluginRegistry::new();
    let effective_plugins = config.effective_plugins();

    for plugin_cfg in &effective_plugins {
        match plugin_cfg.plugin_type.as_str() {
            "openclaw" => {
                let mut p = plugin::openclaw::OpenClawPlugin::new(plugin_cfg, Arc::clone(&chat_state));
                p.set_gateway_event_tx(gateway_event_tx.clone());
                plugin_registry.register(Box::new(p));
            }
            "ollama" => {
                plugin_registry.register(Box::new(plugin::ollama::OllamaPlugin::new(plugin_cfg, Arc::clone(&chat_state))));
            }
            "openai-compatible" => {
                plugin_registry.register(Box::new(plugin::openai_compat::OpenAICompatPlugin::new(plugin_cfg, Arc::clone(&chat_state))));
            }
            "n8n" => {
                plugin_registry.register(Box::new(plugin::n8n::N8nPlugin::new(plugin_cfg, Arc::clone(&chat_state))));
            }
            "mcp" => {
                plugin_registry.register(Box::new(plugin::mcp::McpPlugin::new(plugin_cfg, Arc::clone(&chat_state))));
            }
            other => {
                tracing::warn!(plugin_type = other, "unknown plugin type — skipping");
            }
        }
    }

    // Set active plugin ID on chat state
    if let Some(active_id) = plugin_registry.active_id() {
        if let Ok(mut cs) = chat_state.lock() {
            cs.active_plugin_id = Some(active_id.to_string());
            // Load saved history for the initial conversation
            cs.load_from_history(&chat_history);
        }
    }

    // Connect all plugins (OpenClaw plugins spawn gateway tasks)
    plugin_registry.connect_all();

    let plugin_names = plugin_registry.names();
    let plugin_count = plugin_registry.len();

    // ── Backward compat: also spawn gateway connections the old way
    // for connections that are NOT covered by plugins.
    let effective_connections = config.effective_connections();
    let connection_names: Vec<String> = effective_connections.iter().map(|c| c.name.clone()).collect();

    // If we have plugins, the OpenClaw plugins already handle gateway connections.
    // Only spawn the old way if there are NO plugins configured.
    let (gateway_count, gateway_cmd_tx) = if effective_plugins.is_empty() {
        gateway::spawn_all_connections(
            &effective_connections,
            gateway_event_tx,
            Arc::clone(&chat_state),
        )
        .await
    } else {
        // OpenClaw plugins handle their own gateway connections.
        // Count OpenClaw plugins as gateway connections.
        let oc_count = effective_plugins
            .iter()
            .filter(|p| p.plugin_type == "openclaw")
            .count();
        (oc_count, plugin_registry.active_command_sender().map(|tx| {
            // Create a GatewayCommand sender that wraps the PluginCommand sender
            let (gw_tx, mut gw_rx) = mpsc::unbounded_channel::<gateway::GatewayCommand>();
            let plugin_tx = tx;
            tokio::spawn(async move {
                while let Some(cmd) = gw_rx.recv().await {
                    let plugin_cmd = match cmd {
                        gateway::GatewayCommand::SendChat { message, session_key, attachments } => {
                            plugin::PluginCommand::SendChat { message, session_key, attachments }
                        }
                        gateway::GatewayCommand::ListSessions => plugin::PluginCommand::ListSessions,
                    };
                    let _ = plugin_tx.send(plugin_cmd);
                }
            });
            gw_tx
        }))
    };
    let gateway_enabled = gateway_count > 0;

    info!(
        gateway_count,
        plugin_count,
        plugins = ?plugin_names,
        connections = ?connection_names,
        "plugins and gateway connections initialized"
    );

    spawn_monitor(
        config.clone(),
        monitor_cmd_rx,
        status_tx,
        if gateway_enabled {
            Some(gateway_monitor_rx)
        } else {
            None
        },
        gateway_enabled,
    );

    let mut tray = TrayState::new(
        tray_cmd_tx,
        config.widget.auto_restart,
        autostart::effective_autostart(&config),
        config.widget.notifications,
        &connection_names,
    )?;
    tray.set_gateway_configured(gateway_enabled)?;

    // Initialize tray plugin items
    let plugin_statuses = plugin_registry.plugin_statuses();
    tray.init_plugin_items(&plugin_statuses);

    // Register global hotkey: Cmd+Shift+O (macOS) / Ctrl+Shift+O (Win/Linux)
    let _hotkey_manager = {
        use global_hotkey::{
            hotkey::{Code, HotKey, Modifiers},
            GlobalHotKeyManager,
        };
        let manager = GlobalHotKeyManager::new();
        match manager {
            Ok(mgr) => {
                let modifiers = if cfg!(target_os = "macos") {
                    Modifiers::SUPER | Modifiers::SHIFT
                } else {
                    Modifiers::CONTROL | Modifiers::SHIFT
                };
                let hotkey = HotKey::new(Some(modifiers), Code::KeyO);
                match mgr.register(hotkey) {
                    Ok(()) => info!("global hotkey registered: Cmd/Ctrl+Shift+O"),
                    Err(e) => tracing::warn!("hotkey registration failed (another app may use it): {e}"),
                }
                Some(mgr)
            }
            Err(e) => {
                tracing::warn!("hotkey manager init failed: {e}");
                None
            }
        }
    };

    let mut last_tailscale_check = std::time::Instant::now();

    loop {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
            };
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        tray.poll_menu_events();

        // Poll global hotkey events
        if let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
                let _ = tray_cmd_tx_hotkey.send(TrayCommand::OpenChat);
            }
        }

        while let Ok(event) = gateway_event_rx.try_recv() {
            tray.handle_gateway_event(&event)?;
            let _ = gateway_monitor_tx.send(event.clone());

            // Track latency and push log entries
            match &event {
                gateway::GatewayEvent::Latency { connection_name, ms } => {
                    latency_tracker.push(*ms);
                    if let Ok(mut cs) = chat_state.lock() {
                        cs.add_log(
                            dashboard::LogLevel::Info,
                            connection_name,
                            &format!("Latency: {}ms", ms),
                        );
                    }
                }
                gateway::GatewayEvent::Connected { connection_name, .. } => {
                    if let Ok(mut cs) = chat_state.lock() {
                        cs.add_log(
                            dashboard::LogLevel::Info,
                            connection_name,
                            "Connected",
                        );
                    }
                    // Notify on reconnection (if window not focused)
                    if config.widget.notifications {
                        let focused = chat_state.lock().map(|cs| cs.window_focused).unwrap_or(true);
                        if !focused && notification_limiter.allow(connection_name) {
                            tray::send_notification_public(&format!("{connection_name} reconnected"));
                        }
                    }
                }
                gateway::GatewayEvent::Disconnected { connection_name, reason } => {
                    if let Ok(mut cs) = chat_state.lock() {
                        cs.add_log(
                            dashboard::LogLevel::Warn,
                            connection_name,
                            &format!("Disconnected: {}", reason),
                        );
                    }
                    // Notify on disconnect
                    if config.widget.notifications && notification_limiter.allow(connection_name) {
                        tray::send_notification_public(&format!("{connection_name} disconnected"));
                    }
                }
                gateway::GatewayEvent::Error { connection_name, message } => {
                    if let Ok(mut cs) = chat_state.lock() {
                        cs.add_log(
                            dashboard::LogLevel::Error,
                            connection_name,
                            message,
                        );
                    }
                    // Notify on error
                    if config.widget.notifications && notification_limiter.allow(connection_name) {
                        tray::send_notification_public(&format!("{connection_name} error: {message}"));
                    }
                }
                gateway::GatewayEvent::NodeStatus { connection_name, online, node_name, .. } => {
                    if let Ok(mut cs) = chat_state.lock() {
                        let status = if *online { "online" } else { "offline" };
                        let name = node_name.as_deref().unwrap_or("unknown");
                        cs.add_log(
                            dashboard::LogLevel::Info,
                            connection_name,
                            &format!("Node {} is {}", name, status),
                        );
                    }
                }
            }
        }

        // Update dashboard data periodically (max 1/sec)
        if last_dashboard_push.elapsed() >= std::time::Duration::from_secs(1) {
            let statuses = plugin_registry.plugin_statuses();
            let plugin_types: Vec<(String, String, String)> = plugin_registry
                .all()
                .iter()
                .map(|p| (p.id().0.clone(), p.plugin_type().to_string(), p.icon().to_string()))
                .collect();
            let dash_data = dashboard::build_dashboard_data(
                &statuses,
                &plugin_types,
                &latency_tracker,
                start_time,
                Some(&health_tracker),
            );
            if let Ok(mut cs) = chat_state.lock() {
                cs.dashboard_data = dash_data;
            }
            last_dashboard_push = std::time::Instant::now();
        }

        // Chat notifications: when chat window is NOT open, notify on new replies
        if let Ok(mut cs) = chat_state.lock() {
            if !cs.window_open {
                // Extract reply events from inbox
                let mut replies = Vec::new();
                let mut remaining = Vec::new();
                for event in cs.inbox.drain(..) {
                    match event {
                        chat::ChatInbound::Reply { text, agent_name, .. } => {
                            replies.push((text, agent_name));
                        }
                        other => remaining.push(other),
                    }
                }
                cs.inbox = remaining;

                for (text, agent_name) in replies {
                    let agent = agent_name.as_deref().unwrap_or("Agent");
                    let preview: String = text.chars().take(100).collect();
                    if config.widget.notifications {
                        tray::send_chat_notification(
                            &format!("{agent} replied: {preview}"),
                            &tray_cmd_tx_hotkey,
                        );
                    }
                    let name = agent_name.unwrap_or_else(|| "Agent".to_string());
                    cs.messages.push(chat::ChatMessage {
                        sender: chat::ChatSender::Agent(name.clone()),
                        text: text.clone(),
                    });
                    while cs.messages.len() > 50 {
                        cs.messages.remove(0);
                    }
                    cs.waiting_for_reply = false;

                    // Persist to history
                    chat_history.push_message(
                        &cs.conversation_key(),
                        history::PersistedMessage {
                            sender: "agent".to_string(),
                            agent_name: Some(name),
                            text,
                        },
                    );
                }
            }
        }

        // Debounced history save (every 2 seconds)
        if last_history_save.elapsed() >= std::time::Duration::from_secs(2) {
            chat_history.save_if_dirty();
            last_history_save = std::time::Instant::now();
        }

        while let Ok(update) = status_rx.try_recv() {
            tray.update_status(
                update.node_status,
                &update.detail,
                update.pid,
                update.crash_loop,
                update.stop_reason,
            )?;
            info!(
                "status update: node={:?} crash_loop={} detail={}",
                update.node_status, update.crash_loop, update.detail
            );
        }

        while let Ok(cmd) = tray_cmd_rx.try_recv() {
            match cmd {
                TrayCommand::Refresh => {
                    let _ = monitor_cmd_tx.send(MonitorCommand::Refresh);
                }
                TrayCommand::RestartNode => {
                    let _ = monitor_cmd_tx.send(MonitorCommand::RestartNode);
                }
                TrayCommand::StopNode => {
                    let _ = monitor_cmd_tx.send(MonitorCommand::StopNode);
                }
                TrayCommand::ToggleAutoRestart(enabled) => {
                    let _ = monitor_cmd_tx.send(MonitorCommand::SetAutoRestart(enabled));
                    tray.set_auto_restart(enabled);
                }
                TrayCommand::ToggleAutoStart(enabled) => match autostart::set_autostart(enabled) {
                    Ok(()) => {
                        tray.set_auto_start(enabled);
                    }
                    Err(err) => {
                        error!("failed to toggle autostart: {err}");
                        tray.set_auto_start(!enabled);
                    }
                },
                TrayCommand::OpenGatewayUi => {
                    // Open the first connection's gateway UI
                    let conns = config.effective_connections();
                    if let Some(conn) = conns.first() {
                        let http_url = conn.gateway_url.replace("ws://", "http://").replace("wss://", "https://");
                        info!("opening gateway UI: {http_url}");
                        let _ = open::that(&http_url);
                    }
                }
                TrayCommand::ViewLogs => {
                    let base = dirs::home_dir()
                        .map(|h| h.join(".openclaw"))
                        .unwrap_or_default();
                    let logs_dir = base.join("logs");
                    let target = if logs_dir.exists() { logs_dir } else { base };
                    info!("opening logs dir: {}", target.display());
                    let _ = open::that(&target);
                }
                TrayCommand::Settings => {
                    if let Some(saved_config) = settings::run_settings_window(&config)? {
                        config = saved_config;
                        tray.set_auto_restart(config.widget.auto_restart);
                        tray.set_auto_start(autostart::effective_autostart(&config));
                    }
                }
                TrayCommand::SetupWizard => {
                    if let Some(saved_config) = wizard::run_setup_wizard(&config)? {
                        config = saved_config;
                        tray.set_auto_start(autostart::effective_autostart(&config));
                    }
                }
                TrayCommand::CheckForUpdates => {
                    let update_tx = tray_cmd_tx2.clone();
                    tokio::spawn(async move {
                        match update::check_for_updates().await {
                            Some((version, url)) => {
                                let body = format!("{} {version}\n{url}", i18n::t("notif_update_available"));
                                tray::send_notification_public(&body);
                                let _ = update_tx.send(TrayCommand::ShowDownloadButton(version));
                            }
                            None => {
                                tray::send_notification_public(i18n::t("notif_up_to_date"));
                            }
                        }
                    });
                }
                TrayCommand::DownloadUpdate(tag) => {
                    tokio::spawn(async move {
                        match update::download_and_install(&tag).await {
                            Ok(()) => {
                                tray::send_notification_public(
                                    "Update installed — restart to apply",
                                );
                            }
                            Err(e) => {
                                error!("update download failed: {e}");
                                tray::send_notification_public(&format!(
                                    "Update failed: {e}"
                                ));
                            }
                        }
                    });
                }
                TrayCommand::OpenChat => {
                    // Prefer plugin command senders map; fall back to gateway cmd
                    let senders = plugin_registry.command_senders();
                    if !senders.is_empty() {
                        chat::run_chat_window_plugin(
                            Arc::clone(&chat_state),
                            senders,
                        )?;
                    } else if let Some(ref cmd_tx) = gateway_cmd_tx {
                        chat::run_chat_window(
                            Arc::clone(&chat_state),
                            cmd_tx.clone(),
                        )?;
                    } else {
                        tray::send_notification_public("No gateway connection for chat");
                    }
                }
                TrayCommand::CopyDiagnostics => {
                    let conns = config.effective_connections();
                    let diag = tray.collect_diagnostics(&conns);
                    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(diag)) {
                        Ok(()) => {
                            tray::send_notification_public(i18n::t("diagnostics_copied"));
                        }
                        Err(e) => {
                            error!("clipboard copy failed: {e}");
                            tray::send_notification_public(&format!("Copy failed: {e}"));
                        }
                    }
                }
                TrayCommand::ShowDownloadButton(tag) => {
                    tray.show_download_update(&tag);
                }
                TrayCommand::Uninstall => {
                    if let Ok(true) = uninstall::confirm_uninstall() {
                        let _ = uninstall::perform_uninstall();
                        tray::send_notification_public(i18n::t("notif_uninstalled"));
                        return Ok(());
                    }
                }
                TrayCommand::Exit => return Ok(()),
            }
        }

        // Periodic plugin health checks (every 60s)
        if last_health_check.elapsed() >= std::time::Duration::from_secs(60) {
            let results = plugin_registry.health_check_all();
            for (plugin_id, health) in &results {
                health_tracker.record(plugin_id, health.clone());
            }
            // Update tray plugin statuses after health check
            let statuses = plugin_registry.plugin_statuses();
            tray.update_plugin_statuses(&statuses);
            last_health_check = std::time::Instant::now();
        }

        // Periodic Tailscale status check (every 60s)
        if last_tailscale_check.elapsed() >= std::time::Duration::from_secs(60) {
            let gw_urls: Vec<String> = config.effective_connections().iter().map(|c| c.gateway_url.clone()).collect();
            tray.update_tailscale_status(&gw_urls);
            last_tailscale_check = std::time::Instant::now();
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn run_daemon(config: Config) -> error::Result<()> {
    let (_tray_cmd_tx, _tray_cmd_rx) = mpsc::unbounded_channel::<TrayCommand>();
    let (_monitor_cmd_tx, monitor_cmd_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();
    let (gateway_event_tx, mut gateway_event_rx) = mpsc::unbounded_channel();
    let (gateway_monitor_tx, gateway_monitor_rx) = mpsc::unbounded_channel();

    let daemon_chat_state = Arc::new(Mutex::new(chat::ChatState::new()));
    let effective_connections = config.effective_connections();
    let (gateway_count, _gateway_cmd_tx) = gateway::spawn_all_connections(
        &effective_connections,
        gateway_event_tx,
        daemon_chat_state,
    )
    .await;
    let gateway_enabled = gateway_count > 0;

    spawn_monitor(
        config,
        monitor_cmd_rx,
        status_tx,
        if gateway_enabled {
            Some(gateway_monitor_rx)
        } else {
            None
        },
        gateway_enabled,
    );

    loop {
        tokio::select! {
            Some(event) = gateway_event_rx.recv() => {
                let _ = gateway_monitor_tx.send(event);
            }
            Some(update) = status_rx.recv() => {
                println!(
                    "Node: {:?}, CrashLoop: {}, Detail: {}",
                    update.node_status, update.crash_loop, update.detail
                );
            }
            else => break,
        }
    }

    Ok(())
}

fn run_status() -> error::Result<i32> {
    let config = Config::load()?;
    let config_path = config_path()?;
    let autostart = autostart::effective_autostart(&config);

    println!("OpenClaw Node Widget v{}", env!("CARGO_PKG_VERSION"));
    println!("Config: {}", config_path.display());

    match process::detect_node() {
        Ok(Some(proc_info)) => println!("Node: Online (PID {})", proc_info.pid),
        Ok(None) => println!("Node: Offline"),
        Err(err) => println!("Node: Unknown ({err})"),
    }

    let conns = config.effective_connections();
    if conns.is_empty() {
        println!("Gateway: (not configured)");
    } else {
        for conn in &conns {
            println!("Connection [{}]: {}", conn.name, conn.gateway_url);
        }
    }

    println!(
        "Auto-restart: {}",
        if config.widget.auto_restart {
            "on"
        } else {
            "off"
        }
    );
    println!("Auto-start: {}", if autostart { "on" } else { "off" });

    let code = match process::detect_node() {
        Ok(Some(_)) => 0,
        _ => 1,
    };
    Ok(code)
}
