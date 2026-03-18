use std::{
    future::pending,
    time::{Duration, Instant},
};

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{config::Config, gateway::GatewayEvent, process};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Online,
    Offline,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Manual,
    CrashLoop,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct StatusUpdate {
    pub node_status: NodeStatus,
    pub pid: Option<i32>,
    pub detail: String,
    pub crash_loop: bool,
    pub stop_reason: StopReason,
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
    status_tx: mpsc::UnboundedSender<StatusUpdate>,
    mut gateway_rx: Option<mpsc::UnboundedReceiver<GatewayEvent>>,
    gateway_configured: bool,
) {
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(Duration::from_secs(config.widget.check_interval_secs));
        let mut offline_count = 0u32;
        let mut offline_since: Option<Instant> = None;
        let mut stop_cooldown_until: Option<Instant> = None;
        let mut restart_failures = 0u32;
        let mut auto_restart = config.widget.auto_restart;
        let mut crash_loop = false;
        let mut stop_reason = StopReason::Unknown;

        let mut gateway_connected = false;
        let mut gateway_online: Option<bool> = None;
        let mut gateway_error: Option<String> = None;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let status = if gateway_configured && gateway_connected {
                        status_from_gateway(gateway_online, gateway_error.as_deref(), crash_loop, stop_reason)
                    } else {
                        let status = status_from_process(stop_reason, crash_loop);
                        if status.node_status == NodeStatus::Offline {
                            offline_count = offline_count.saturating_add(1);
                            if offline_since.is_none() {
                                offline_since = Some(Instant::now());
                            }

                            if !crash_loop && restart_failures >= config.widget.max_restart_attempts {
                                crash_loop = true;
                                stop_reason = StopReason::CrashLoop;
                                warn!("node crash loop detected: max restart attempts reached");
                            }

                            if !crash_loop {
                                if let Some(since) = offline_since {
                                    if since.elapsed().as_secs() >= config.widget.crash_loop_secs {
                                        crash_loop = true;
                                        stop_reason = StopReason::CrashLoop;
                                        warn!("node crash loop detected: offline threshold exceeded");
                                    }
                                }
                            }
                        } else if status.node_status == NodeStatus::Online {
                            offline_count = 0;
                            offline_since = None;
                            restart_failures = 0;
                            crash_loop = false;
                            stop_reason = StopReason::Unknown;
                        }

                        if should_restart(
                            auto_restart,
                            crash_loop,
                            offline_count,
                            config.widget.restart_threshold,
                            stop_cooldown_until,
                            restart_failures,
                            config.widget.max_restart_attempts,
                        ) {
                            match process::start_node(&config) {
                                Ok(()) => {
                                    info!("node restart triggered by monitor");
                                    offline_count = 0;
                                    offline_since = None;
                                    stop_reason = StopReason::Unknown;
                                }
                                Err(err) => {
                                    restart_failures = restart_failures.saturating_add(1);
                                    warn!("node restart failed: {err}");
                                }
                            }
                        }

                        status
                    };

                    let _ = status_tx.send(StatusUpdate {
                        node_status: status.node_status,
                        pid: status.pid,
                        detail: status.detail,
                        crash_loop,
                        stop_reason,
                    });
                }
                maybe_event = recv_gateway_event(&mut gateway_rx), if gateway_configured => {
                    let Some(event) = maybe_event else {
                        gateway_rx = None;
                        gateway_connected = false;
                        gateway_online = None;
                        gateway_error = Some("gateway event channel closed".to_string());
                        continue;
                    };

                    match event {
                        GatewayEvent::Connected => {
                            gateway_connected = true;
                            gateway_error = None;
                        }
                        GatewayEvent::Disconnected(reason) => {
                            gateway_connected = false;
                            gateway_online = None;
                            gateway_error = Some(reason);
                        }
                        GatewayEvent::NodeStatus { online } => {
                            gateway_connected = true;
                            gateway_online = Some(online);
                            gateway_error = None;

                            let _ = status_tx.send(status_from_gateway(gateway_online, None, crash_loop, stop_reason));
                        }
                        GatewayEvent::Error(message) => {
                            gateway_connected = false;
                            gateway_online = None;
                            gateway_error = Some(message.clone());
                            let _ = status_tx.send(StatusUpdate {
                                node_status: NodeStatus::Unknown,
                                pid: None,
                                detail: format!("Gateway error: {message}"),
                                crash_loop,
                                stop_reason,
                            });
                        }
                    }
                }
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        MonitorCommand::Refresh => {
                            let _ = status_tx.send(StatusUpdate {
                                node_status: NodeStatus::Unknown,
                                pid: None,
                                detail: "Refreshing...".to_string(),
                                crash_loop,
                                stop_reason,
                            });
                        }
                        MonitorCommand::RestartNode => {
                            crash_loop = false;
                            stop_reason = StopReason::Unknown;
                            restart_failures = 0;
                            stop_cooldown_until = None;
                            offline_since = None;

                            if let Err(err) = process::stop_node() {
                                warn!("manual stop before restart failed: {err}");
                            }
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            match process::start_node(&config) {
                                Ok(()) => {
                                    info!("node manually restarted");
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
                                    stop_reason = StopReason::Manual;
                                    info!("node manually stopped, cooldown {}s", config.widget.restart_cooldown_secs);
                                }
                                Err(err) => warn!("manual stop failed: {err}"),
                            }
                        }
                        MonitorCommand::SetAutoRestart(enabled) => {
                            auto_restart = enabled;
                            info!("auto_restart set to {enabled}");
                        }
                    }
                }
                else => break,
            }
        }
    });
}

async fn recv_gateway_event(
    gateway_rx: &mut Option<mpsc::UnboundedReceiver<GatewayEvent>>,
) -> Option<GatewayEvent> {
    match gateway_rx.as_mut() {
        Some(rx) => rx.recv().await,
        None => pending::<Option<GatewayEvent>>().await,
    }
}

fn status_from_gateway(
    gateway_online: Option<bool>,
    gateway_error: Option<&str>,
    crash_loop: bool,
    stop_reason: StopReason,
) -> StatusUpdate {
    let status = match gateway_online {
        Some(true) => NodeStatus::Online,
        Some(false) => NodeStatus::Offline,
        None => NodeStatus::Unknown,
    };

    let detail = if let Some(error) = gateway_error {
        format!("Gateway disconnected: {error}")
    } else {
        match status {
            NodeStatus::Online => "Online".to_string(),
            NodeStatus::Offline => "Offline".to_string(),
            NodeStatus::Unknown => "Checking...".to_string(),
        }
    };

    StatusUpdate {
        node_status: status,
        pid: None,
        detail,
        crash_loop,
        stop_reason,
    }
}

fn status_from_process(stop_reason: StopReason, crash_loop: bool) -> StatusUpdate {
    let mut detail = "Offline".to_string();
    let (status, pid) = match process::detect_node() {
        Ok(Some(proc_info)) => {
            detail = "Online".to_string();
            (NodeStatus::Online, Some(proc_info.pid))
        }
        Ok(None) => (NodeStatus::Offline, None),
        Err(err) => {
            warn!("process detection failed: {err}");
            detail = format!("Error: {err}");
            (NodeStatus::Unknown, None)
        }
    };

    if crash_loop {
        detail = "Node crash loop detected - auto-restart paused".to_string();
    } else if status == NodeStatus::Offline && stop_reason == StopReason::Manual {
        detail = "Node stopped (manual)".to_string();
    }

    StatusUpdate {
        node_status: status,
        pid,
        detail,
        crash_loop,
        stop_reason,
    }
}

fn should_restart(
    auto_restart: bool,
    crash_loop: bool,
    offline_count: u32,
    threshold: u32,
    cooldown: Option<Instant>,
    restart_failures: u32,
    max_restart_attempts: u32,
) -> bool {
    if !auto_restart || crash_loop || offline_count < threshold {
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
