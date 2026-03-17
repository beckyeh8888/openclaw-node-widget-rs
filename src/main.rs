mod autostart;
mod config;
mod error;
mod lock;
mod monitor;
mod process;
mod setup;
mod tray;

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::{
    config::{config_path, Config},
    lock::{try_acquire_lock, AcquireResult},
    monitor::{spawn_monitor, MonitorCommand},
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
    let command = cli.command.unwrap_or(Commands::Run);

    let mut config = Config::load()?;

    if let Some(url) = cli.gateway {
        config.gateway.url = url;
    }
    if let Some(token) = cli.token {
        config.gateway.token = token;
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

            setup::maybe_run_setup(&mut config)?;
            init_tracing(&config);
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

            setup::maybe_run_setup(&mut config)?;
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
            setup::force_run_setup(&mut config)?;
            Ok(0)
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
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();

    spawn_monitor(config.clone(), monitor_cmd_rx, status_tx);

    let mut tray = TrayState::new(
        tray_cmd_tx,
        config.widget.auto_restart,
        autostart::effective_autostart(&config),
        config.widget.notifications,
    )?;

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
                TrayCommand::Settings => {
                    if let Ok(path) = config::config_path() {
                        info!("opening config: {}", path.display());
                        let _ = open::that(&path);
                    }
                }
                TrayCommand::Exit => return Ok(()),
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn run_daemon(config: Config) -> error::Result<()> {
    let (_tray_cmd_tx, _tray_cmd_rx) = mpsc::unbounded_channel::<TrayCommand>();
    let (_monitor_cmd_tx, monitor_cmd_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();

    spawn_monitor(config, monitor_cmd_rx, status_tx);

    while let Some(update) = status_rx.recv().await {
        println!(
            "Node: {:?}, CrashLoop: {}, Detail: {}",
            update.node_status, update.crash_loop, update.detail
        );
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

    println!("Gateway: {}", config.gateway.url);
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
