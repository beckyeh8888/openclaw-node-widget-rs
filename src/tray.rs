use image::ImageFormat;
use notify_rust::Notification;
use tokio::sync::mpsc;
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

use crate::{
    error::{AppError, Result},
    gateway::GatewayEvent,
    i18n::t,
    monitor::{NodeStatus, StopReason},
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
    Uninstall,
    Exit,
}

pub struct TrayState {
    tray: TrayIcon,
    status_item: MenuItem,
    conn_detail_item: MenuItem,
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
    gateway_version: Option<String>,
    node_name: Option<String>,
    connected_at: Option<std::time::Instant>,
}

impl TrayState {
    pub fn new(
        cmd_tx: mpsc::UnboundedSender<TrayCommand>,
        auto_restart: bool,
        auto_start: bool,
        notifications_enabled: bool,
    ) -> Result<Self> {
        let menu = Menu::new();
        let a = |item: &dyn tray_icon::menu::IsMenuItem| -> Result<()> {
            menu.append(item).map_err(|e| AppError::Tray(e.to_string()))
        };
        let sep = || PredefinedMenuItem::separator();

        let status_item = MenuItem::new(format!("Node: {}", t("status_unknown")), false, None);
        let conn_detail_item = MenuItem::new(format!("{}{}", t("connection_details"), t("na")), false, None);
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
        let uninstall_item = MenuItem::new(t("uninstall"), true, None);
        let exit_item = MenuItem::new(t("exit"), true, None);

        a(&status_item)?;
        a(&conn_detail_item)?;
        a(&sep())?;
        a(&refresh_item)?;
        a(&restart_item)?;
        a(&stop_item)?;
        a(&sep())?;
        a(&open_gateway_item)?;
        a(&view_logs_item)?;
        a(&sep())?;
        a(&auto_restart_item)?;
        a(&auto_start_item)?;
        a(&sep())?;
        a(&settings_item)?;
        a(&setup_wizard_item)?;
        a(&check_updates_item)?;
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

        let refresh_id = refresh_item.id().clone();
        let restart_id = restart_item.id().clone();
        let stop_id = stop_item.id().clone();
        let open_gateway_id = open_gateway_item.id().clone();
        let view_logs_id = view_logs_item.id().clone();
        let settings_id = settings_item.id().clone();
        let setup_wizard_id = setup_wizard_item.id().clone();
        let check_updates_id = check_updates_item.id().clone();
        let uninstall_id = uninstall_item.id().clone();
        let exit_id = exit_item.id().clone();
        let auto_restart_id = auto_restart_item.id().clone();
        let auto_start_id = auto_start_item.id().clone();

        Ok(Self {
            tray,
            status_item,
            conn_detail_item,
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
            gateway_version: None,
            node_name: None,
            connected_at: None,
        })
    }

    pub fn update_status(
        &mut self,
        status: NodeStatus,
        detail: &str,
        _pid: Option<i32>,
        crash_loop: bool,
        stop_reason: StopReason,
    ) -> Result<()> {
        let status_text = if crash_loop {
            "Crash Loop".to_string()
        } else if status == NodeStatus::Offline && stop_reason == StopReason::Manual {
            "Stopped".to_string()
        } else {
            detail.to_string()
        };

        let label = format!("Node: {status_text}");

        self.status_item.set_text(&label);
        self.tray
            .set_icon(Some(icon_for_status(status)?))
            .map_err(|e| AppError::Tray(e.to_string()))?;
        let tooltip = match status {
            NodeStatus::Online => format!("OpenClaw Node: Online\nGateway: {}", self.gateway_status),
            NodeStatus::Offline => format!("OpenClaw Node: Offline\nGateway: {}", self.gateway_status),
            NodeStatus::Unknown => format!("OpenClaw Node: Checking...\nGateway: {}", self.gateway_status),
        };
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
            GatewayEvent::Connected { gateway_version } => {
                self.gateway_status = t("gateway_connected").to_string();
                self.gateway_version = gateway_version.clone();
                self.connected_at = Some(std::time::Instant::now());
            }
            GatewayEvent::Disconnected(reason) => {
                self.gateway_status = format!("Disconnected: {reason}");
                self.connected_at = None;
            }
            GatewayEvent::NodeStatus { online, node_name } => {
                self.gateway_status = if *online {
                    t("gateway_connected").to_string()
                } else {
                    t("gateway_node_offline").to_string()
                };
                if node_name.is_some() {
                    self.node_name = node_name.clone();
                }
            }
            GatewayEvent::Error(message) => {
                self.gateway_status = format!("Error: {message}");
                self.connected_at = None;
            }
        };
        self.update_conn_detail();
        self.refresh_tooltip()
    }

    fn update_conn_detail(&mut self) {
        let gw = self.gateway_version.as_deref().unwrap_or(t("na"));
        let node = self.node_name.as_deref().unwrap_or(t("na"));
        let uptime = match self.connected_at {
            Some(at) => {
                let secs = at.elapsed().as_secs();
                if secs < 60 {
                    t("just_now").to_string()
                } else if secs < 3600 {
                    format!("{}{}",secs / 60, t("minutes_short"))
                } else {
                    format!("{}{} {}{}", secs / 3600, t("hours_short"), (secs % 3600) / 60, t("minutes_short"))
                }
            }
            None => t("na").to_string(),
        };
        self.conn_detail_item.set_text(&format!("GW:{gw} | {node} | {uptime}"));
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
            if previous == NodeStatus::Online && status == NodeStatus::Offline {
                send_notification("OpenClaw Node went offline");
            }
            if previous == NodeStatus::Offline && status == NodeStatus::Online {
                send_notification("OpenClaw Node is online");
            }
        }
    }

    fn refresh_tooltip(&mut self) -> Result<()> {
        self.tray
            .set_tooltip(Some(format!(
                "OpenClaw Node: {}\nGateway: {}",
                self.last_node_tooltip, self.gateway_status
            )))
            .map_err(|e| AppError::Tray(e.to_string()))?;
        Ok(())
    }
}

fn icon_for_status(status: NodeStatus) -> Result<Icon> {
    let bytes = match status {
        NodeStatus::Online => include_bytes!("../assets/icon_online.png").as_slice(),
        NodeStatus::Offline => include_bytes!("../assets/icon_offline.png").as_slice(),
        NodeStatus::Unknown => include_bytes!("../assets/icon_unknown.png").as_slice(),
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
    use std::process::Command;

    // Escape single quotes for PowerShell
    let escaped = body.replace('\'', "''");
    let script = format!(
        r#"[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null;
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom, ContentType = WindowsRuntime] | Out-Null;
$xml = [Windows.Data.Xml.Dom.XmlDocument]::new();
$xml.LoadXml('<toast><visual><binding template="ToastGeneric"><text>OpenClaw Node Widget</text><text>{escaped}</text></binding></visual><audio silent="false"/></toast>');
$toast = [Windows.UI.Notifications.ToastNotification]::new($xml);
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('OpenClaw.NodeWidget').Show($toast)"#
    );

    match Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn()
    {
        Ok(_) => tracing::debug!("windows toast sent: {body}"),
        Err(e) => {
            tracing::warn!("windows toast failed: {e}, falling back to notify-rust");
            let _ = Notification::new()
                .appname("OpenClaw Node Widget")
                .summary("OpenClaw Node Widget")
                .body(body)
                .show();
        }
    }
}
