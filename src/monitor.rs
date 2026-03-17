use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{
    config::Config,
    gateway::{ConnectionState, GatewayEvent},
    process,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Online,
    Offline,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct StatusUpdate {
    pub node_status: NodeStatus,
    pub connection_state: ConnectionState,
    pub pid: Option<i32>,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub enum MonitorCommand {
    Refresh,
    RestartNode,
    StopNode,
    SetAutoRestart(bool),
}

pub fn spawn_monitor(
    config: Config,
    mut command_rx: mpsc::UnboundedReceiver<MonitorCommand>,
    mut gateway_rx: mpsc::UnboundedReceiver<GatewayEvent>,
    status_tx: mpsc::UnboundedSender<StatusUpdate>,
) {
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(Duration::from_secs(config.widget.check_interval_secs));
        let mut connection_state = ConnectionState::Disconnected;
        let mut gateway_node_online: Option<bool> = None;
        let mut offline_count = 0u32;
        let mut stop_cooldown_until: Option<Instant> = None;
        let mut restart_failures = 0u32;
        let mut auto_restart = config.widget.auto_restart;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let Some(snapshot) = evaluate_status(connection_state, gateway_node_online) else {
                        continue;
                    };

                    let mut effective = snapshot;
                    if connection_state != ConnectionState::Connected {
                        match process::detect_node() {
                            Ok(Some(proc_info)) => {
                                effective.node_status = NodeStatus::Online;
                                effective.pid = Some(proc_info.pid);
                                effective.detail = "Online (no gateway)".to_string();
                            }
                            Ok(None) => {
                                effective.node_status = NodeStatus::Offline;
                                effective.pid = None;
                                effective.detail = "Offline".to_string();
                            }
                            Err(err) => warn!("process detection failed: {err}"),
                        }
                    }

                    if effective.node_status == NodeStatus::Offline {
                        offline_count = offline_count.saturating_add(1);
                    } else {
                        offline_count = 0;
                        restart_failures = 0;
                    }

                    if should_restart(auto_restart, offline_count, config.widget.restart_threshold, stop_cooldown_until, restart_failures, config.widget.max_restart_attempts) {
                        match process::start_node(&config) {
                            Ok(()) => {
                                info!("node restart triggered by monitor");
                                offline_count = 0;
                            }
                            Err(err) => {
                                restart_failures = restart_failures.saturating_add(1);
                                warn!("node restart failed: {err}");
                            }
                        }
                    }

                    let _ = status_tx.send(effective);
                }
                Some(event) = gateway_rx.recv() => {
                    match event {
                        GatewayEvent::ConnectionState(state) => {
                            connection_state = state;
                            let _ = status_tx.send(StatusUpdate {
                                node_status: NodeStatus::Unknown,
                                connection_state,
                                pid: None,
                                detail: format!("Gateway: {:?}", connection_state),
                            });
                        }
                        GatewayEvent::NodeOnline(online) => {
                            gateway_node_online = Some(online);
                        }
                    }
                }
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        MonitorCommand::Refresh => {
                            let _ = status_tx.send(StatusUpdate {
                                node_status: NodeStatus::Unknown,
                                connection_state,
                                pid: None,
                                detail: "Refreshing...".to_string(),
                            });
                        }
                        MonitorCommand::RestartNode => {
                            if let Err(err) = process::stop_node() {
                                warn!("manual stop before restart failed: {err}");
                            }
                            match process::start_node(&config) {
                                Ok(()) => {
                                    restart_failures = 0;
                                    stop_cooldown_until = None;
                                }
                                Err(err) => {
                                    restart_failures = restart_failures.saturating_add(1);
                                    warn!("manual restart failed: {err}");
                                }
                            }
                        }
                        MonitorCommand::StopNode => {
                            match process::stop_node() {
                                Ok(()) => {
                                    stop_cooldown_until = Some(Instant::now() + Duration::from_secs(config.widget.restart_cooldown_secs));
                                }
                                Err(err) => warn!("manual stop failed: {err}"),
                            }
                        }
                        MonitorCommand::SetAutoRestart(enabled) => {
                            auto_restart = enabled;
                        }
                    }
                }
                else => break,
            }
        }
    });
}

fn evaluate_status(
    connection_state: ConnectionState,
    gateway_node_online: Option<bool>,
) -> Option<StatusUpdate> {
    match connection_state {
        ConnectionState::Connected => {
            let node_status = match gateway_node_online {
                Some(true) => NodeStatus::Online,
                Some(false) => NodeStatus::Offline,
                None => NodeStatus::Unknown,
            };

            let detail = match node_status {
                NodeStatus::Online => "Online".to_string(),
                NodeStatus::Offline => "Offline".to_string(),
                NodeStatus::Unknown => "Unknown".to_string(),
            };

            Some(StatusUpdate {
                node_status,
                connection_state,
                pid: None,
                detail,
            })
        }
        ConnectionState::Connecting | ConnectionState::Disconnected => Some(StatusUpdate {
            node_status: NodeStatus::Unknown,
            connection_state,
            pid: None,
            detail: "Gateway unavailable".to_string(),
        }),
    }
}

fn should_restart(
    auto_restart: bool,
    offline_count: u32,
    threshold: u32,
    cooldown: Option<Instant>,
    restart_failures: u32,
    max_restart_attempts: u32,
) -> bool {
    if !auto_restart || offline_count < threshold {
        return false;
    }

    if restart_failures >= max_restart_attempts {
        return false;
    }

    if let Some(until) = cooldown {
        return Instant::now() >= until;
    }

    true
}
