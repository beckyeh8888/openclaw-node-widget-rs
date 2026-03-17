use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{config::Config, process};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Online,
    Offline,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct StatusUpdate {
    pub node_status: NodeStatus,
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
    status_tx: mpsc::UnboundedSender<StatusUpdate>,
) {
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(Duration::from_secs(config.widget.check_interval_secs));
        let mut offline_count = 0u32;
        let mut stop_cooldown_until: Option<Instant> = None;
        let mut restart_failures = 0u32;
        let mut auto_restart = config.widget.auto_restart;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let (status, pid, detail) = match process::detect_node() {
                        Ok(Some(proc_info)) => {
                            (NodeStatus::Online, Some(proc_info.pid), format!("Online (PID {})", proc_info.pid))
                        }
                        Ok(None) => {
                            (NodeStatus::Offline, None, "Offline".to_string())
                        }
                        Err(err) => {
                            warn!("process detection failed: {err}");
                            (NodeStatus::Unknown, None, format!("Error: {err}"))
                        }
                    };

                    if status == NodeStatus::Offline {
                        offline_count = offline_count.saturating_add(1);
                    } else {
                        offline_count = 0;
                        restart_failures = 0;
                    }

                    // Auto-restart logic
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

                    let _ = status_tx.send(StatusUpdate {
                        node_status: status,
                        pid,
                        detail,
                    });
                }
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        MonitorCommand::Refresh => {
                            let _ = status_tx.send(StatusUpdate {
                                node_status: NodeStatus::Unknown,
                                pid: None,
                                detail: "Refreshing...".to_string(),
                            });
                        }
                        MonitorCommand::RestartNode => {
                            if let Err(err) = process::stop_node() {
                                warn!("manual stop before restart failed: {err}");
                            }
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            match process::start_node(&config) {
                                Ok(()) => {
                                    restart_failures = 0;
                                    stop_cooldown_until = None;
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
