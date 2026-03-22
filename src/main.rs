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
mod media;
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

fn main() {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    match rt.block_on(run()) {
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
    if let Ok(mut cs) = chat_state.lock() {
        cs.media_store = media::MediaStore::new();
    }
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
                            Some(plugin::PluginCommand::SendChat { message, session_key, attachments })
                        }
                        gateway::GatewayCommand::ListSessions => Some(plugin::PluginCommand::ListSessions),
                        gateway::GatewayCommand::ListAgents => {
                            // ListAgents is handled by the gateway directly, not plugins
                            None
                        }
                    };
                    if let Some(cmd) = plugin_cmd {
                        let _ = plugin_tx.send(cmd);
                    }
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

    // ── Build chat command senders for webview IPC ──────────────
    let chat_cmd_senders: Arc<std::collections::HashMap<String, mpsc::UnboundedSender<plugin::PluginCommand>>> = {
        let senders = plugin_registry.command_senders();
        if !senders.is_empty() {
            Arc::new(senders)
        } else if let Some(ref cmd_tx) = gateway_cmd_tx {
            let (plugin_tx, mut plugin_rx) = mpsc::unbounded_channel::<plugin::PluginCommand>();
            let gw_tx = cmd_tx.clone();
            tokio::spawn(async move {
                while let Some(cmd) = plugin_rx.recv().await {
                    match cmd {
                        plugin::PluginCommand::SendChat { message, session_key, attachments } => {
                            let _ = gw_tx.send(gateway::GatewayCommand::SendChat { message, session_key, attachments });
                        }
                        plugin::PluginCommand::ListSessions => {
                            let _ = gw_tx.send(gateway::GatewayCommand::ListSessions);
                        }
                    }
                }
            });
            let mut m = std::collections::HashMap::new();
            m.insert("default".to_string(), plugin_tx);
            Arc::new(m)
        } else {
            Arc::new(std::collections::HashMap::new())
        }
    };

    // ── WebView / Window created on demand (first OpenChat) ────
    let mut chat_window: Option<tao::window::Window> = None;
    let mut chat_webview: Option<wry::WebView> = None;

    // ── Unified tao event loop ─────────────────────────────────
    use tao::event::{Event, StartCause, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};

    let event_loop = EventLoopBuilder::new().build();

    event_loop.run(move |event, elwt, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        );

        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. })
            | Event::MainEventsCleared => {
                // ── Tray menu events ──────────────────────────
                tray.poll_menu_events();

                // ── Global hotkey ─────────────────────────────
                if let Ok(hk_event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
                    if hk_event.state == global_hotkey::HotKeyState::Pressed {
                        let _ = tray_cmd_tx_hotkey.send(TrayCommand::OpenChat);
                    }
                }

                // ── Gateway events ────────────────────────────
                while let Ok(event) = gateway_event_rx.try_recv() {
                    if let Err(e) = tray.handle_gateway_event(&event) {
                        error!("tray gateway event error: {e}");
                    }
                    let _ = gateway_monitor_tx.send(event.clone());

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

                // ── Dashboard data (max 1/sec) ────────────────
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

                // ── Chat: webview events OR background notifications ─
                if chat_webview.is_some() {
                    // Show notifications for replies while window is hidden
                    let window_open = chat_state.lock().map(|s| s.window_open).unwrap_or(false);
                    if !window_open && config.widget.notifications {
                        if let Ok(cs) = chat_state.lock() {
                            for inbox_event in &cs.inbox {
                                if let chat::ChatInbound::Reply { text, agent_name, .. } = inbox_event {
                                    let agent = agent_name.as_deref().unwrap_or("Agent");
                                    let preview: String = text.chars().take(100).collect();
                                    tray::send_chat_notification(
                                        &format!("{agent} replied: {preview}"),
                                        &tray_cmd_tx_hotkey,
                                    );
                                }
                            }
                        }
                    }
                    // Forward inbox to webview (keeps it in sync even when hidden)
                    if let (Some(ref wv), Some(ref w)) = (&chat_webview, &chat_window) {
                        chat::process_chat_events(&chat_state, wv, w);
                    }
                } else {
                    // No webview — handle reply notifications in main loop
                    if let Ok(mut cs) = chat_state.lock() {
                        if !cs.window_open {
                            let mut replies = Vec::new();
                            let mut remaining = Vec::new();
                            for inbox_event in cs.inbox.drain(..) {
                                match inbox_event {
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
                                    media_path: None,
                                    media_type: None,
                                });
                                while cs.messages.len() > 50 {
                                    cs.messages.remove(0);
                                }
                                cs.waiting_for_reply = false;

                                chat_history.push_message(
                                    &cs.conversation_key(),
                                    history::PersistedMessage {
                                        sender: "agent".to_string(),
                                        agent_name: Some(name),
                                        text,
                                        media_path: None,
                                        media_type: None,
                                        created_at: {
                                            std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_millis() as i64)
                                                .unwrap_or(0)
                                        },
                                    },
                                );
                            }
                        }
                    }
                }

                // ── History save (every 2s) ───────────────────
                if last_history_save.elapsed() >= std::time::Duration::from_secs(2) {
                    if let Ok(cs) = chat_state.lock() {
                        cs.save_to_history(&mut chat_history);
                    }
                    chat_history.save_if_dirty();
                    last_history_save = std::time::Instant::now();
                }

                // ── Status updates ────────────────────────────
                while let Ok(update) = status_rx.try_recv() {
                    if let Err(e) = tray.update_status(
                        update.node_status,
                        &update.detail,
                        update.pid,
                        update.crash_loop,
                        update.stop_reason,
                    ) {
                        error!("tray status update error: {e}");
                    }
                    info!(
                        "status update: node={:?} crash_loop={} detail={}",
                        update.node_status, update.crash_loop, update.detail
                    );
                }

                // ── Tray commands ─────────────────────────────
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
                            match settings::run_settings_window(&config) {
                                Ok(Some(saved_config)) => {
                                    config = saved_config;
                                    tray.set_auto_restart(config.widget.auto_restart);
                                    tray.set_auto_start(autostart::effective_autostart(&config));
                                }
                                Ok(None) => {}
                                Err(e) => error!("settings window error: {e}"),
                            }
                        }
                        TrayCommand::SetupWizard => {
                            match wizard::run_setup_wizard(&config) {
                                Ok(Some(saved_config)) => {
                                    config = saved_config;
                                    tray.set_auto_start(autostart::effective_autostart(&config));
                                }
                                Ok(None) => {}
                                Err(e) => error!("setup wizard error: {e}"),
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
                            if chat_cmd_senders.is_empty() {
                                tray::send_notification_public("No gateway connection for chat");
                            } else if chat_window.is_none() {
                                // First open: create Window + WebView
                                let always_on_top = config.widget.always_on_top;
                                match tao::window::WindowBuilder::new()
                                    .with_title("\u{1f916} OpenClaw Chat")
                                    .with_inner_size(tao::dpi::LogicalSize::new(420.0, 620.0))
                                    .with_min_inner_size(tao::dpi::LogicalSize::new(380.0, 400.0))
                                    .with_always_on_top(always_on_top)
                                    .build(elwt)
                                {
                                    Ok(w) => {
                                        match chat::create_chat_webview(
                                            &w,
                                            &chat_state,
                                            Arc::clone(&chat_cmd_senders),
                                        ) {
                                            Ok(wv) => {
                                                if let Ok(mut state) = chat_state.lock() {
                                                    state.window_open = true;
                                                    state.window_focused = true;
                                                }
                                                chat_window = Some(w);
                                                chat_webview = Some(wv);
                                            }
                                            Err(e) => {
                                                error!("failed to create webview: {e}");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("failed to create chat window: {e}");
                                    }
                                }
                            } else if let Some(ref w) = chat_window {
                                // Re-show existing window
                                w.set_visible(true);
                                if let Ok(mut state) = chat_state.lock() {
                                    state.window_open = true;
                                    state.window_focused = true;
                                }
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
                                std::process::exit(0);
                            }
                        }
                        TrayCommand::Exit => {
                            std::process::exit(0);
                        }
                    }
                }

                // ── Plugin health checks (every 60s) ─────────
                if last_health_check.elapsed() >= std::time::Duration::from_secs(60) {
                    let results = plugin_registry.health_check_all();
                    for (plugin_id, health) in &results {
                        health_tracker.record(plugin_id, health.clone());
                    }
                    let statuses = plugin_registry.plugin_statuses();
                    tray.update_plugin_statuses(&statuses);
                    last_health_check = std::time::Instant::now();
                }

                // ── Tailscale status (every 60s) ──────────────
                if last_tailscale_check.elapsed() >= std::time::Duration::from_secs(60) {
                    let gw_urls: Vec<String> = config.effective_connections().iter().map(|c| c.gateway_url.clone()).collect();
                    tray.update_tailscale_status(&gw_urls);
                    last_tailscale_check = std::time::Instant::now();
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                // Hide the chat window instead of destroying it
                if let Some(ref w) = chat_window {
                    w.set_visible(false);
                }
                if let Ok(mut state) = chat_state.lock() {
                    state.window_open = false;
                    state.window_focused = false;
                }
            }
            _ => {}
        }
    });
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
