use image::ImageFormat;
use tokio::sync::mpsc;
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

use crate::{
    error::{AppError, Result},
    monitor::NodeStatus,
};

#[derive(Debug, Clone)]
pub enum TrayCommand {
    Refresh,
    RestartNode,
    StopNode,
    ToggleAutoRestart(bool),
    ToggleAutoStart(bool),
    Settings,
    Exit,
}

pub struct TrayState {
    tray: TrayIcon,
    status_item: MenuItem,
    auto_restart_item: CheckMenuItem,
    auto_start_item: CheckMenuItem,
    refresh_id: MenuId,
    restart_id: MenuId,
    stop_id: MenuId,
    settings_id: MenuId,
    exit_id: MenuId,
    auto_restart_id: MenuId,
    auto_start_id: MenuId,
    cmd_tx: mpsc::UnboundedSender<TrayCommand>,
}

impl TrayState {
    pub fn new(
        cmd_tx: mpsc::UnboundedSender<TrayCommand>,
        auto_restart: bool,
        auto_start: bool,
    ) -> Result<Self> {
        let menu = Menu::new();

        let status_item = MenuItem::new("Status: Unknown", false, None);
        let refresh_item = MenuItem::new("Refresh", true, None);
        let restart_item = MenuItem::new("Restart Node", true, None);
        let stop_item = MenuItem::new("Stop Node", true, None);
        let auto_restart_item = CheckMenuItem::new("Auto-restart", true, auto_restart, None);
        let auto_start_item = CheckMenuItem::new("Auto-start", true, auto_start, None);
        let settings_item = MenuItem::new("Settings", true, None);
        let exit_item = MenuItem::new("Exit", true, None);

        menu.append(&status_item)
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&refresh_item)
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&restart_item)
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&stop_item)
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&auto_restart_item)
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&auto_start_item)
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&settings_item)
            .map_err(|e| AppError::Tray(e.to_string()))?;
        menu.append(&exit_item)
            .map_err(|e| AppError::Tray(e.to_string()))?;

        let icon = icon_for_status(NodeStatus::Unknown)?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("OpenClaw Node: Unknown")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()
            .map_err(|e| AppError::Tray(e.to_string()))?;

        let refresh_id = refresh_item.id().clone();
        let restart_id = restart_item.id().clone();
        let stop_id = stop_item.id().clone();
        let settings_id = settings_item.id().clone();
        let exit_id = exit_item.id().clone();
        let auto_restart_id = auto_restart_item.id().clone();
        let auto_start_id = auto_start_item.id().clone();

        Ok(Self {
            tray,
            status_item,
            auto_restart_item,
            auto_start_item,
            refresh_id,
            restart_id,
            stop_id,
            settings_id,
            exit_id,
            auto_restart_id,
            auto_start_id,
            cmd_tx,
        })
    }

    pub fn update_status(
        &mut self,
        status: NodeStatus,
        detail: &str,
        pid: Option<i32>,
    ) -> Result<()> {
        let label = match pid {
            Some(pid) => format!("Status: {detail} (PID {pid})"),
            None => format!("Status: {detail}"),
        };

        self.status_item.set_text(&label);
        self.tray
            .set_icon(Some(icon_for_status(status)?))
            .map_err(|e| AppError::Tray(e.to_string()))?;
        self.tray
            .set_tooltip(Some(format!("OpenClaw Node: {detail}")))
            .map_err(|e| AppError::Tray(e.to_string()))?;
        Ok(())
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
        if id == self.refresh_id {
            let _ = self.cmd_tx.send(TrayCommand::Refresh);
            return;
        }
        if id == self.restart_id {
            let _ = self.cmd_tx.send(TrayCommand::RestartNode);
            return;
        }
        if id == self.stop_id {
            let _ = self.cmd_tx.send(TrayCommand::StopNode);
            return;
        }
        if id == self.settings_id {
            let _ = self.cmd_tx.send(TrayCommand::Settings);
            return;
        }
        if id == self.exit_id {
            let _ = self.cmd_tx.send(TrayCommand::Exit);
            return;
        }
        if id == self.auto_restart_id {
            let checked = !self.auto_restart_item.is_checked();
            let _ = self.cmd_tx.send(TrayCommand::ToggleAutoRestart(checked));
            return;
        }
        if id == self.auto_start_id {
            let checked = !self.auto_start_item.is_checked();
            let _ = self.cmd_tx.send(TrayCommand::ToggleAutoStart(checked));
        }
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
