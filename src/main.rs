mod autostart;
mod config;
mod error;
mod gateway;
mod monitor;
mod process;
mod tray;

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::{
    config::Config,
    gateway::spawn_gateway,
    monitor::{spawn_monitor, MonitorCommand, NodeStatus},
    tray::{TrayCommand, TrayState},
};

#[derive(Debug, Parser)]
#[command(name = "openclaw-node-widget")]
#[command(about = "OpenClaw Node tray widget")]
struct Cli {
    #[arg(long)]
    gateway: Option<String>,
    #[arg(long)]
    token: Option<String>,
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
    if let Err(err) = run().await {
        error!("{err}");
        std::process::exit(1);
    }
}

async fn run() -> error::Result<()> {
    let cli = Cli::parse();
    let mut config = Config::load()?;

    if let Some(url) = cli.gateway {
        config.gateway.url = url;
    }
    if let Some(token) = cli.token {
        config.gateway.token = token;
    }

    init_tracing(&config);

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run_with_tray(config).await,
        Commands::Daemon => run_daemon(config).await,
        Commands::Status => {
            run_status().await;
            Ok(())
        }
        Commands::Stop => {
            process::stop_node()?;
            println!("Node stopped");
            Ok(())
        }
        Commands::Restart => {
            let _ = process::stop_node();
            process::start_node(&config)?;
            println!("Node restarted");
            Ok(())
        }
        Commands::Config => {
            println!("{}", toml::to_string_pretty(&config).unwrap_or_default());
            Ok(())
        }
        Commands::Setup => {
            println!("setup wizard is not implemented in Phase 1");
            config.save()?;
            Ok(())
        }
    }
}

fn init_tracing(config: &Config) {
    let level = config.log.level.clone();
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn run_with_tray(config: Config) -> error::Result<()> {
    let (tray_cmd_tx, mut tray_cmd_rx) = mpsc::unbounded_channel();
    let (monitor_cmd_tx, monitor_cmd_rx) = mpsc::unbounded_channel();
    let (gateway_tx, gateway_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();

    spawn_gateway(config.clone(), gateway_tx);
    spawn_monitor(config.clone(), monitor_cmd_rx, gateway_rx, status_tx);

    let mut tray = TrayState::new(
        tray_cmd_tx,
        config.widget.auto_restart,
        config.startup.auto_start || autostart::is_autostart_enabled(),
    )?;

    loop {
        tray.poll_menu_events();

        while let Ok(update) = status_rx.try_recv() {
            tray.update_status(update.node_status, &update.detail, update.pid)?;
            info!(
                "status update: node={:?} gateway={:?} detail={}",
                update.node_status, update.connection_state, update.detail
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
                    tray.set_auto_restart(enabled)?;
                }
                TrayCommand::ToggleAutoStart(enabled) => match autostart::set_autostart(enabled) {
                    Ok(()) => {
                        tray.set_auto_start(enabled)?;
                    }
                    Err(err) => {
                        error!("failed to toggle autostart: {err}");
                        tray.set_auto_start(!enabled)?;
                    }
                },
                TrayCommand::Settings => {
                    info!("settings action requested");
                }
                TrayCommand::Exit => return Ok(()),
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn run_daemon(config: Config) -> error::Result<()> {
    let (_tray_cmd_tx, _tray_cmd_rx) = mpsc::unbounded_channel::<TrayCommand>();
    let (_monitor_cmd_tx, monitor_cmd_rx) = mpsc::unbounded_channel();
    let (gateway_tx, gateway_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();

    spawn_gateway(config.clone(), gateway_tx);
    spawn_monitor(config, monitor_cmd_rx, gateway_rx, status_tx);

    while let Some(update) = status_rx.recv().await {
        println!(
            "Node: {:?}, Gateway: {:?}, Detail: {}",
            update.node_status, update.connection_state, update.detail
        );
    }

    Ok(())
}

async fn run_status() {
    match process::detect_node() {
        Ok(Some(proc_info)) => {
            println!("Node: Online (PID {})", proc_info.pid);
            std::process::exit(0);
        }
        Ok(None) => {
            println!("Node: Offline");
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("status check failed: {err}");
            std::process::exit(1);
        }
    }
}
