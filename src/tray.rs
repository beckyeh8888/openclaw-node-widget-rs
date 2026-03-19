use std::collections::HashMap;

use image::ImageFormat;
use notify_rust::Notification;
use tokio::sync::mpsc;
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

use crate::{
    error::{AppError, Result},
    gateway::{GatewayEvent, GatewayStats},
    i18n::t,
    monitor::{NodeStatus, StopReason},
    plugin::ConnectionStatus as PluginConnectionStatus,
    tailscale,
};

#[derive(Debug, Clone)]
pub enum TrayCommand {
    Refresh,
    RestartNode,
    StopNode,
    ToggleAutoRestart(bool),
    ToggleAutoStart(bool),
    OpenGatewayUi,
    ViewLogs,
    Settings,
    SetupWizard,
    CheckForUpdates,
    DownloadUpdate(String),
    ShowDownloadButton(String),
    OpenChat,
    CopyDiagnostics,
    Uninstall,
    Exit,
}

/// Per-connection state tracked in the tray
struct ConnectionTrayState {
    menu_item: MenuItem,
    gateway_version: Option<String>,
    node_name: Option<String>,
    connected_at: Option<std::time::Instant>,
    last_connected: Option<chrono::DateTime<chrono::Local>>,
    last_error: Option<String>,
    online: Option<bool>,
    stats: GatewayStats,
    status_label: String,
}

pub struct TrayState {
    tray: TrayIcon,
    status_item: MenuItem,
    conn_detail_item: MenuItem,
    last_error_item: MenuItem,
    last_connected_item: MenuItem,
    auto_restart_item: CheckMenuItem,
    auto_start_item: CheckMenuItem,
    refresh_id: MenuId,
    restart_id: MenuId,
    stop_id: MenuId,
    open_gateway_id: MenuId,
    view_logs_id: MenuId,
    settings_id: MenuId,
    setup_wizard_id: MenuId,
    check_updates_id: MenuId,
    download_update_item: MenuItem,
    download_update_id: MenuId,
    pending_update_tag: Option<String>,
    chat_id: MenuId,
    copy_diagnostics_id: MenuId,
    uninstall_id: MenuId,
    exit_id: MenuId,
    auto_restart_id: MenuId,
    auto_start_id: MenuId,
    cmd_tx: mpsc::UnboundedSender<TrayCommand>,
    notifications_enabled: bool,
    last_status: Option<NodeStatus>,
    last_crash_loop: bool,
    gateway_status: String,
    last_node_tooltip: String,
    stats_sessions_item: MenuItem,
    stats_errors_item: MenuItem,
    stats_activity_item: MenuItem,
    /// Per-connection state, keyed by connection name
    connections: HashMap<String, ConnectionTrayState>,
    /// Ordered connection names (for deterministic display)
    connection_order: Vec<String>,
    multi_connection: bool,
    tailscale_item: MenuItem,
    tailscale_status: tailscale::TailscaleStatus,
    latency_item: MenuItem,
    latency_ms: Option<u64>,
    /// Plugin status menu items, keyed by plugin id.
    plugin_items: HashMap<String, MenuItem>,
    plugin_order: Vec<String>,
}

impl TrayState {
    pub fn new(
        cmd_tx: mpsc::UnboundedSender<TrayCommand>,
        auto_restart: bool,
        auto_start: bool,
        notifications_enabled: bool,
        connection_names: &[String],
    ) -> Result<Self> {
        let menu = Menu::new();
        let a = |item: &dyn tray_icon::menu::IsMenuItem| -> Result<()> {
            menu.append(item).map_err(|e| AppError::Tray(e.to_string()))
        };
        let sep = || PredefinedMenuItem::separator();

        let multi_connection = connection_names.len() > 1;

        // Per-connection status items (only shown for multi-connection)
        let mut connections = HashMap::new();
        let mut connection_order = Vec::new();
        if multi_connection {
            for name in connection_names {
                let label = format!("{}: {}", name, t("status_unknown"));
                let item = MenuItem::new(&label, false, None);
                a(&item)?;
                connections.insert(
                    name.clone(),
                    ConnectionTrayState {
                        menu_item: item,
                        gateway_version: None,
                        node_name: None,
                        connected_at: None,
                        last_connected: None,
                        last_error: None,
                        online: None,
                        stats: GatewayStats::default(),
                        status_label: t("status_unknown").to_string(),
                    },
                );
                connection_order.push(name.clone());
            }
            a(&sep())?;
        } else if connection_names.len() == 1 {
            // Single connection — create entry without extra menu item
            let name = &connection_names[0];
            connections.insert(
                name.clone(),
                ConnectionTrayState {
                    menu_item: MenuItem::new("", false, None), // unused for single
                    gateway_version: None,
                    node_name: None,
                    connected_at: None,
                    last_connected: None,
                    last_error: None,
                    online: None,
                    stats: GatewayStats::default(),
                    status_label: t("status_unknown").to_string(),
                },
            );
            connection_order.push(name.clone());
        }

        let status_item = MenuItem::new(format!("Node: {}", t("status_unknown")), false, None);
        let conn_detail_item = MenuItem::new(format!("{}{}", t("connection_details"), t("na")), false, None);
        let last_error_item = MenuItem::new(format!("{}{}", t("last_error_label"), t("none")), false, None);
        let last_connected_item = MenuItem::new(format!("{}{}", t("last_connected_label"), t("na")), false, None);
        let stats_sessions_item = MenuItem::new(format!("{}{}", t("stats_sessions"), "0"), false, None);
        let stats_errors_item = MenuItem::new(format!("{}{}", t("stats_errors_24h"), "0"), false, None);
        let stats_activity_item = MenuItem::new(format!("{}{}", t("stats_last_activity"), t("na")), false, None);
        let initial_ts = tailscale::check_status();
        let ts_label = match initial_ts {
            tailscale::TailscaleStatus::Connected => t("tailscale_connected"),
            tailscale::TailscaleStatus::Disconnected => t("tailscale_disconnected"),
            tailscale::TailscaleStatus::NotInstalled => t("tailscale_not_installed"),
        };
        let tailscale_item = MenuItem::new(ts_label, false, None);
        let latency_item = MenuItem::new(t("latency_na"), false, None);
        let chat_item = MenuItem::new(format!("\u{1F4AC} {}", t("chat")), true, None);
        let refresh_item = MenuItem::new(t("refresh"), true, None);
        let restart_item = MenuItem::new(t("restart_node"), true, None);
        let stop_item = MenuItem::new(t("stop_node"), true, None);
        let open_gateway_item = MenuItem::new(t("open_gateway_ui"), true, None);
        let view_logs_item = MenuItem::new(t("view_logs"), true, None);
        let auto_restart_item = CheckMenuItem::new(t("auto_restart"), true, auto_restart, None);
        let auto_start_item = CheckMenuItem::new(t("auto_start"), true, auto_start, None);
        let settings_item = MenuItem::new(t("settings"), true, None);
        let setup_wizard_item = MenuItem::new(t("setup_wizard"), true, None);
        let check_updates_item = MenuItem::new(t("check_for_updates"), true, None);
        let download_update_item = MenuItem::new(t("no_updates"), false, None);
        let copy_diagnostics_item = MenuItem::new(t("copy_diagnostics"), true, None);
        let uninstall_item = MenuItem::new(t("uninstall"), true, None);
        let exit_item = MenuItem::new(t("exit"), true, None);

        a(&status_item)?;
        a(&conn_detail_item)?;
        a(&last_error_item)?;
        a(&last_connected_item)?;
        a(&stats_sessions_item)?;
        a(&stats_errors_item)?;
        a(&stats_activity_item)?;
        a(&tailscale_item)?;
        a(&latency_item)?;
        a(&sep())?;
        a(&refresh_item)?;
        a(&restart_item)?;
        a(&stop_item)?;
        a(&sep())?;
        a(&chat_item)?;
        a(&open_gateway_item)?;
        a(&view_logs_item)?;
        a(&sep())?;
        a(&auto_restart_item)?;
        a(&auto_start_item)?;
        a(&sep())?;
        a(&settings_item)?;
        a(&setup_wizard_item)?;
        a(&check_updates_item)?;
        a(&download_update_item)?;
        a(&copy_diagnostics_item)?;
        a(&sep())?;
        a(&uninstall_item)?;
        a(&exit_item)?;

        let icon = icon_for_status(NodeStatus::Unknown)?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("OpenClaw Node: Unknown\nGateway: Not configured")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()
            .map_err(|e| AppError::Tray(e.to_string()))?;

        let chat_id = chat_item.id().clone();
        let refresh_id = refresh_item.id().clone();
        let restart_id = restart_item.id().clone();
        let stop_id = stop_item.id().clone();
        let open_gateway_id = open_gateway_item.id().clone();
        let view_logs_id = view_logs_item.id().clone();
        let settings_id = settings_item.id().clone();
        let setup_wizard_id = setup_wizard_item.id().clone();
        let check_updates_id = check_updates_item.id().clone();
        let download_update_id = download_update_item.id().clone();
        let copy_diagnostics_id = copy_diagnostics_item.id().clone();
        let uninstall_id = uninstall_item.id().clone();
        let exit_id = exit_item.id().clone();
        let auto_restart_id = auto_restart_item.id().clone();
        let auto_start_id = auto_start_item.id().clone();

        Ok(Self {
            tray,
            status_item,
            conn_detail_item,
            last_error_item,
            last_connected_item,
            auto_restart_item,
            auto_start_item,
            refresh_id,
            restart_id,
            stop_id,
            open_gateway_id,
            view_logs_id,
            settings_id,
            setup_wizard_id,
            check_updates_id,
            download_update_item,
            download_update_id,
            pending_update_tag: None,
            chat_id,
            copy_diagnostics_id,
            uninstall_id,
            exit_id,
            auto_restart_id,
            auto_start_id,
            cmd_tx,
            notifications_enabled,
            last_status: None,
            last_crash_loop: false,
            gateway_status: t("gateway_not_configured").to_string(),
            last_node_tooltip: t("status_unknown").to_string(),
            stats_sessions_item,
            stats_errors_item,
            stats_activity_item,
            connections,
            connection_order,
            multi_connection,
            tailscale_item,
            tailscale_status: initial_ts,
            latency_item,
            latency_ms: None,
            plugin_items: HashMap::new(),
            plugin_order: Vec::new(),
        })
    }

    pub fn update_status(
        &mut self,
        status: NodeStatus,
        detail: &str,
        _pid: Option<i32>,
        crash_loop: bool,
        _stop_reason: StopReason,
    ) -> Result<()> {
        let status_text = if crash_loop {
            t("status_crash_loop").to_string()
        } else {
            match status {
                NodeStatus::Online => t("status_online").to_string(),
                NodeStatus::Offline => t("status_offline").to_string(),
                NodeStatus::Stopped => t("status_stopped").to_string(),
                NodeStatus::GatewayDown => t("status_gateway_down").to_string(),
                NodeStatus::AuthFailed => t("status_auth_failed").to_string(),
                NodeStatus::Reconnecting => t("status_reconnecting").to_string(),
                NodeStatus::Unknown => detail.to_string(),
            }
        };

        let label = format!("Node: {status_text}");

        self.status_item.set_text(&label);
        self.tray
            .set_icon(Some(icon_for_status(status)?))
            .map_err(|e| AppError::Tray(e.to_string()))?;
        let tooltip_status = match status {
            NodeStatus::Online => t("status_online"),
            NodeStatus::Offline => t("status_offline"),
            NodeStatus::Stopped => t("status_stopped"),
            NodeStatus::GatewayDown => t("status_gateway_down"),
            NodeStatus::AuthFailed => t("status_auth_failed"),
            NodeStatus::Reconnecting => t("status_reconnecting"),
            NodeStatus::Unknown => t("status_checking"),
        };
        let tooltip = format!("OpenClaw Node: {tooltip_status}\nGateway: {}", self.gateway_status);
        self.tray
            .set_tooltip(Some(tooltip))
            .map_err(|e| AppError::Tray(e.to_string()))?;

        self.notify_transitions(status, crash_loop);
        self.last_status = Some(status);
        self.last_crash_loop = crash_loop;
        self.last_node_tooltip = status_text.clone();

        Ok(())
    }

    pub fn set_gateway_configured(&mut self, configured: bool) -> Result<()> {
        self.gateway_status = if configured {
            "Connecting...".to_string()
        } else {
            "Not configured".to_string()
        };
        self.refresh_tooltip()
    }

    pub fn handle_gateway_event(&mut self, event: &GatewayEvent) -> Result<()> {
        match event {
            GatewayEvent::Connected { connection_name, gateway_version } => {
                if let Some(cs) = self.connections.get_mut(connection_name) {
                    cs.gateway_version = gateway_version.clone();
                    cs.connected_at = Some(std::time::Instant::now());
                    cs.last_connected = Some(chrono::Local::now());
                    cs.status_label = t("gateway_connected").to_string();
                }
                self.gateway_status = t("gateway_connected").to_string();
            }
            GatewayEvent::Disconnected { connection_name, reason } => {
                if let Some(cs) = self.connections.get_mut(connection_name) {
                    cs.connected_at = None;
                    cs.last_error = Some(truncate_error(reason));
                    cs.online = None;
                    cs.status_label = format!("Disconnected: {}", truncate_error(reason));
                }
                self.gateway_status = format!("Disconnected: {reason}");
            }
            GatewayEvent::NodeStatus { connection_name, online, node_name, stats } => {
                if let Some(cs) = self.connections.get_mut(connection_name) {
                    cs.online = Some(*online);
                    if node_name.is_some() {
                        cs.node_name = node_name.clone();
                    }
                    cs.stats = stats.clone();
                    cs.status_label = if *online {
                        t("status_online").to_string()
                    } else {
                        t("status_offline").to_string()
                    };
                }
                self.gateway_status = if *online {
                    t("gateway_connected").to_string()
                } else {
                    t("gateway_node_offline").to_string()
                };
            }
            GatewayEvent::Latency { ms, .. } => {
                self.update_latency(Some(*ms));
                if *ms > 500 && self.notifications_enabled {
                    send_notification(t("latency_warning"));
                }
                return Ok(());
            }
            GatewayEvent::Error { connection_name, message } => {
                if let Some(cs) = self.connections.get_mut(connection_name) {
                    cs.connected_at = None;
                    cs.last_error = Some(truncate_error(message));
                    cs.online = None;
                    cs.status_label = format!("Error: {}", truncate_error(message));
                }
                self.gateway_status = format!("Error: {message}");
            }
        };
        self.update_connection_items();
        self.update_conn_detail();
        self.update_diagnostics_items();
        self.update_stats_items();
        self.refresh_tooltip()
    }

    fn update_connection_items(&mut self) {
        if !self.multi_connection {
            return;
        }
        for name in &self.connection_order {
            if let Some(cs) = self.connections.get(name) {
                cs.menu_item.set_text(&format!("{}: {}", name, cs.status_label));
            }
        }
    }

    fn update_conn_detail(&mut self) {
        // For single connection, show its details. For multi, show primary (first) connection.
        let primary = self.connection_order.first().and_then(|n| self.connections.get(n));
        let gw = primary.and_then(|cs| cs.gateway_version.as_deref()).unwrap_or(t("na"));
        let node = primary.and_then(|cs| cs.node_name.as_deref()).unwrap_or(t("na"));
        let uptime = match primary.and_then(|cs| cs.connected_at) {
            Some(at) => {
                let secs = at.elapsed().as_secs();
                if secs < 60 {
                    t("just_now").to_string()
                } else if secs < 3600 {
                    format!("{}{}", secs / 60, t("minutes_short"))
                } else {
                    format!("{}{} {}{}", secs / 3600, t("hours_short"), (secs % 3600) / 60, t("minutes_short"))
                }
            }
            None => t("na").to_string(),
        };
        self.conn_detail_item.set_text(&format!("GW:{gw} | {node} | {uptime}"));
    }

    fn update_diagnostics_items(&mut self) {
        // Show most recent error across all connections
        let last_error = self.connection_order.iter()
            .filter_map(|n| self.connections.get(n))
            .filter_map(|cs| cs.last_error.as_deref())
            .next();
        let error_text = last_error.unwrap_or(t("none"));
        self.last_error_item
            .set_text(&format!("{}{}", t("last_error_label"), error_text));

        // Show most recent connected time
        let last_connected = self.connection_order.iter()
            .filter_map(|n| self.connections.get(n))
            .filter_map(|cs| cs.last_connected)
            .max();
        let connected_text = match last_connected {
            Some(dt) => format_last_connected(dt),
            None => t("na").to_string(),
        };
        self.last_connected_item
            .set_text(&format!("{}{}", t("last_connected_label"), connected_text));
    }

    fn update_stats_items(&mut self) {
        // Aggregate stats across all connections
        let total_sessions: u32 = self.connections.values().map(|cs| cs.stats.active_sessions).sum();
        let total_errors: u32 = self.connections.values().map(|cs| cs.stats.total_errors_24h).sum();
        let last_activity = self.connection_order.iter()
            .filter_map(|n| self.connections.get(n))
            .filter_map(|cs| cs.stats.last_agent_activity.as_deref())
            .next();

        self.stats_sessions_item
            .set_text(&format!("{}{}", t("stats_sessions"), total_sessions));
        self.stats_errors_item
            .set_text(&format!("{}{}", t("stats_errors_24h"), total_errors));
        let activity = last_activity.unwrap_or(t("na"));
        self.stats_activity_item
            .set_text(&format!("{}{}", t("stats_last_activity"), activity));
    }

    pub fn collect_diagnostics(&self, connections: &[crate::config::ConnectionConfig]) -> String {
        let version = env!("CARGO_PKG_VERSION");
        let os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
        let status = self
            .last_status
            .map(|s| format!("{:?}", s))
            .unwrap_or_else(|| "Unknown".to_string());

        let mut conn_lines = String::new();
        for conn in connections {
            let masked_token = conn.gateway_token
                .as_deref()
                .map(mask_token)
                .unwrap_or_else(|| "N/A".to_string());
            let cs_info = self.connections.get(&conn.name)
                .map(|cs| format!("status={}, error={}", cs.status_label, cs.last_error.as_deref().unwrap_or("None")))
                .unwrap_or_else(|| "no state".to_string());
            conn_lines.push_str(&format!(
                "\n  [{name}] url={url} token={token} {info}",
                name = conn.name,
                url = conn.gateway_url,
                token = masked_token,
                info = cs_info,
            ));
        }

        let uptime = self.connection_order.first()
            .and_then(|n| self.connections.get(n))
            .and_then(|cs| cs.connected_at)
            .map(|at| {
                let secs = at.elapsed().as_secs();
                format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
            })
            .unwrap_or_else(|| "N/A".to_string());

        let ts_status = match self.tailscale_status {
            tailscale::TailscaleStatus::Connected => "Connected",
            tailscale::TailscaleStatus::Disconnected => "Disconnected",
            tailscale::TailscaleStatus::NotInstalled => "Not installed",
        };

        let latency_str = match self.latency_ms {
            Some(ms) => format!("{ms}ms"),
            None => "N/A".to_string(),
        };

        format!(
            "OpenClaw Node Widget Diagnostics\n\
             ─────────────────────────────────\n\
             Version:        {version}\n\
             OS:             {os}\n\
             Node Status:    {status}\n\
             Connections:    {count}{conn_lines}\n\
             Uptime:         {uptime}\n\
             Tailscale:      {ts_status}\n\
             Latency:        {latency_str}",
            count = connections.len(),
        )
    }

    /// Update the tray menu with current plugin statuses from the registry.
    pub fn update_plugin_statuses(
        &mut self,
        statuses: &[(String, String, PluginConnectionStatus)],
    ) {
        for (id, name, status) in statuses {
            let status_str = match status {
                PluginConnectionStatus::Connected => "✓",
                PluginConnectionStatus::Disconnected => "✗",
                PluginConnectionStatus::Reconnecting => "↻",
                PluginConnectionStatus::Error(_) => "⚠",
            };
            let label = format!("{status_str} {name}");
            if let Some(item) = self.plugin_items.get(id) {
                item.set_text(&label);
            }
        }
    }

    /// Initialize plugin menu items (call once after plugins are registered).
    pub fn init_plugin_items(
        &mut self,
        plugins: &[(String, String, PluginConnectionStatus)],
    ) {
        // Plugin items are informational (disabled) labels in the menu.
        // We don't dynamically add them to the existing tray menu since
        // tray-icon doesn't support insert-at.  Instead we store them
        // for tooltip / diagnostics use.
        for (id, name, status) in plugins {
            let status_str = match status {
                PluginConnectionStatus::Connected => "✓",
                PluginConnectionStatus::Disconnected => "✗",
                PluginConnectionStatus::Reconnecting => "↻",
                PluginConnectionStatus::Error(_) => "⚠",
            };
            let label = format!("{status_str} {name}");
            let item = MenuItem::new(&label, false, None);
            self.plugin_items.insert(id.clone(), item);
            self.plugin_order.push(id.clone());
        }
    }

    pub fn update_tailscale_status(&mut self, gateway_urls: &[String]) {
        let new_status = tailscale::check_status();
        let label = match new_status {
            tailscale::TailscaleStatus::Connected => t("tailscale_connected"),
            tailscale::TailscaleStatus::Disconnected => t("tailscale_disconnected"),
            tailscale::TailscaleStatus::NotInstalled => t("tailscale_not_installed"),
        };
        self.tailscale_item.set_text(label);

        // Warn if Tailscale went down and a gateway uses a Tailscale IP
        if self.tailscale_status == tailscale::TailscaleStatus::Connected
            && new_status == tailscale::TailscaleStatus::Disconnected
        {
            let uses_ts_ip = gateway_urls.iter().any(|url| tailscale::is_tailscale_ip(url));
            if uses_ts_ip && self.notifications_enabled {
                send_notification(t("tailscale_warning"));
            }
        }
        self.tailscale_status = new_status;
    }

    pub fn update_latency(&mut self, latency_ms: Option<u64>) {
        self.latency_ms = latency_ms;
        let label = match latency_ms {
            Some(ms) => format!("{}{}ms", t("latency_label"), ms),
            None => t("latency_na").to_string(),
        };
        self.latency_item.set_text(&label);
    }

    pub fn show_download_update(&mut self, tag: &str) {
        self.pending_update_tag = Some(tag.to_string());
        self.download_update_item
            .set_text(&format!("⬇ Download {tag}"));
        self.download_update_item.set_enabled(true);
    }

    pub fn set_auto_restart(&mut self, enabled: bool) {
        self.auto_restart_item.set_checked(enabled);
    }

    pub fn set_auto_start(&mut self, enabled: bool) {
        self.auto_start_item.set_checked(enabled);
    }

    pub fn poll_menu_events(&self) {
        let receiver = MenuEvent::receiver();
        while let Ok(event) = receiver.try_recv() {
            self.dispatch_menu_event(event.id);
        }
    }

    fn dispatch_menu_event(&self, id: MenuId) {
        let cmd = if id == self.refresh_id {
            TrayCommand::Refresh
        } else if id == self.restart_id {
            TrayCommand::RestartNode
        } else if id == self.stop_id {
            TrayCommand::StopNode
        } else if id == self.open_gateway_id {
            TrayCommand::OpenGatewayUi
        } else if id == self.view_logs_id {
            TrayCommand::ViewLogs
        } else if id == self.settings_id {
            TrayCommand::Settings
        } else if id == self.setup_wizard_id {
            TrayCommand::SetupWizard
        } else if id == self.check_updates_id {
            TrayCommand::CheckForUpdates
        } else if id == self.download_update_id {
            if let Some(tag) = &self.pending_update_tag {
                TrayCommand::DownloadUpdate(tag.clone())
            } else {
                return;
            }
        } else if id == self.chat_id {
            TrayCommand::OpenChat
        } else if id == self.copy_diagnostics_id {
            TrayCommand::CopyDiagnostics
        } else if id == self.uninstall_id {
            TrayCommand::Uninstall
        } else if id == self.exit_id {
            TrayCommand::Exit
        } else if id == self.auto_restart_id {
            let checked = !self.auto_restart_item.is_checked();
            TrayCommand::ToggleAutoRestart(checked)
        } else if id == self.auto_start_id {
            let checked = !self.auto_start_item.is_checked();
            TrayCommand::ToggleAutoStart(checked)
        } else {
            return;
        };
        let _ = self.cmd_tx.send(cmd);
    }

    fn notify_transitions(&self, status: NodeStatus, crash_loop: bool) {
        if !self.notifications_enabled {
            return;
        }

        if !self.last_crash_loop && crash_loop {
            send_notification("Node crash loop detected");
        }

        if let Some(previous) = self.last_status {
            if previous == NodeStatus::Online && status != NodeStatus::Online {
                send_notification("OpenClaw Node went offline");
            }
            if previous != NodeStatus::Online && status == NodeStatus::Online {
                send_notification("OpenClaw Node is online");
            }
        }
    }

    fn refresh_tooltip(&mut self) -> Result<()> {
        let tooltip = if self.multi_connection {
            let parts: Vec<String> = self.connection_order.iter()
                .filter_map(|n| self.connections.get(n).map(|cs| format!("{}: {}", n, cs.status_label)))
                .collect();
            format!("OpenClaw Node Widget\n{}", parts.join("\n"))
        } else {
            let latency = match self.latency_ms {
                Some(ms) => format!("{}{}ms", t("latency_label"), ms),
                None => String::new(),
            };
            if latency.is_empty() {
                format!(
                    "OpenClaw Node: {}\nGateway: {}",
                    self.last_node_tooltip, self.gateway_status
                )
            } else {
                format!(
                    "OpenClaw Node: {}\nGateway: {}\n{}",
                    self.last_node_tooltip, self.gateway_status, latency
                )
            }
        };
        self.tray
            .set_tooltip(Some(tooltip))
            .map_err(|e| AppError::Tray(e.to_string()))?;
        Ok(())
    }
}

pub use crate::gateway::mask_token;

fn truncate_error(msg: &str) -> String {
    if msg.len() <= 60 {
        msg.to_string()
    } else {
        format!("{}...", &msg[..57])
    }
}

fn format_last_connected(dt: chrono::DateTime<chrono::Local>) -> String {
    let now = chrono::Local::now();
    if now.date_naive() == dt.date_naive() {
        dt.format("%H:%M:%S").to_string()
    } else {
        dt.format("%m-%d %H:%M").to_string()
    }
}

fn icon_for_status(status: NodeStatus) -> Result<Icon> {
    let bytes = match status {
        NodeStatus::Online => include_bytes!("../assets/icon_online.png").as_slice(),
        NodeStatus::Offline | NodeStatus::Stopped | NodeStatus::GatewayDown | NodeStatus::AuthFailed => {
            include_bytes!("../assets/icon_offline.png").as_slice()
        }
        NodeStatus::Reconnecting | NodeStatus::Unknown => {
            include_bytes!("../assets/icon_unknown.png").as_slice()
        }
    };

    icon_from_png(bytes)
}

fn icon_from_png(bytes: &[u8]) -> Result<Icon> {
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .map_err(|e| AppError::Tray(e.to_string()))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).map_err(|e| AppError::Tray(e.to_string()))
}

pub fn send_notification_public(body: &str) {
    send_notification(body);
}

fn send_notification(body: &str) {
    #[cfg(windows)]
    {
        send_notification_windows(body);
        return;
    }

    #[cfg(not(windows))]
    {
        match Notification::new()
            .appname("OpenClaw Node Widget")
            .summary("OpenClaw Node Widget")
            .body(body)
            .show()
        {
            Ok(_) => tracing::debug!("notification sent: {body}"),
            Err(e) => tracing::warn!("notification failed: {e}"),
        }
    }
}

#[cfg(windows)]
fn send_notification_windows(body: &str) {
    use std::os::windows::process::CommandExt;
    use winrt_notification::{Toast, Duration as ToastDuration};

    ensure_start_menu_shortcut();

    const AUM_ID: &str = "OpenClaw.NodeWidget";
    match Toast::new(AUM_ID)
        .title("OpenClaw Node Widget")
        .text1(body)
        .duration(ToastDuration::Short)
        .show()
    {
        Ok(_) => tracing::debug!("windows toast sent: {body}"),
        Err(e) => tracing::warn!("windows toast failed: {e}"),
    }
}

#[cfg(windows)]
fn ensure_start_menu_shortcut() {
    use std::os::windows::process::CommandExt;
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        let lnk = format!(
            "{}\\Microsoft\\Windows\\Start Menu\\Programs\\OpenClaw Node Widget.lnk",
            appdata
        );

        if std::path::Path::new(&lnk).exists() {
            return;
        }

        let exe = std::env::current_exe()
            .unwrap_or_default()
            .display()
            .to_string();

        let script = format!(
            r#"$WshShell = New-Object -ComObject WScript.Shell
$Shortcut = $WshShell.CreateShortcut('{lnk}')
$Shortcut.TargetPath = '{exe}'
$Shortcut.Description = 'OpenClaw Node Widget'
$Shortcut.Save()
$shell = New-Object -ComObject Shell.Application
$dir = $shell.Namespace((Split-Path '{lnk}'))
$item = $dir.ParseName((Split-Path '{lnk}' -Leaf))
$regPath = 'HKCU:\Software\Classes\AppUserModelId\OpenClaw.NodeWidget'
if (-not (Test-Path $regPath)) {{
    New-Item -Path $regPath -Force | Out-Null
    Set-ItemProperty -Path $regPath -Name 'DisplayName' -Value 'OpenClaw Node Widget'
}}"#
        );

        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x08000000)
            .spawn();

        tracing::debug!("registered OpenClaw.NodeWidget AUMID for toast notifications");
    });
}
