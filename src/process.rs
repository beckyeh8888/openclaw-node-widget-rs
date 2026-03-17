use std::{path::PathBuf, process::Stdio, time::Duration};

use sysinfo::{ProcessesToUpdate, System};

use crate::{
    config::Config,
    error::{AppError, Result},
};

#[derive(Debug, Clone)]
pub struct NodeProcessInfo {
    pub pid: i32,
    pub cmdline: String,
}

pub fn detect_node() -> Result<Option<NodeProcessInfo>> {
    let mut system = System::new_all();
    let _ = system.refresh_processes(ProcessesToUpdate::All, true);

    for process in system.processes().values() {
        let cmdline = process
            .cmd()
            .iter()
            .map(|v| v.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();

        if cmdline.contains("openclaw") && cmdline.contains("node") && cmdline.contains("run") {
            return Ok(Some(NodeProcessInfo {
                pid: process.pid().as_u32() as i32,
                cmdline,
            }));
        }
    }

    Ok(None)
}

pub fn start_node(config: &Config) -> Result<()> {
    let mut parts = config.node.command.split_whitespace();
    let binary = parts
        .next()
        .ok_or_else(|| AppError::Process("node.command is empty".to_string()))?;

    let mut cmd = std::process::Command::new(binary);
    cmd.args(parts);
    cmd.args(&config.node.args);

    if let Some(workdir) = working_directory(config) {
        cmd.current_dir(workdir);
    }

    for (key, value) in &config.node.env {
        cmd.env(key, value);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use winapi::um::winbase::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(unix)]
    {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    }

    cmd.stdin(Stdio::null());
    cmd.spawn().map_err(|e| AppError::Process(e.to_string()))?;
    Ok(())
}

pub fn stop_node() -> Result<()> {
    #[cfg(windows)]
    {
        stop_node_windows()
    }

    #[cfg(unix)]
    {
        stop_node_unix()
    }
}

#[cfg(unix)]
fn stop_node_unix() -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let Some(proc_info) = detect_node()? else {
        return Ok(());
    };

    let pid = Pid::from_raw(proc_info.pid);
    kill(pid, Signal::SIGTERM).map_err(|e| AppError::Process(e.to_string()))?;
    std::thread::sleep(Duration::from_secs(2));

    if detect_node()?.is_some() {
        kill(pid, Signal::SIGKILL).map_err(|e| AppError::Process(e.to_string()))?;
    }

    Ok(())
}

#[cfg(windows)]
fn stop_node_windows() -> Result<()> {
    let Some(proc_info) = detect_node()? else {
        return Ok(());
    };

    let status = std::process::Command::new("taskkill")
        .args(["/PID", &proc_info.pid.to_string(), "/F"])
        .status()
        .map_err(|e| AppError::Process(e.to_string()))?;

    if !status.success() {
        return Err(AppError::Process(format!(
            "taskkill failed for pid {}",
            proc_info.pid
        )));
    }

    Ok(())
}

fn working_directory(config: &Config) -> Option<PathBuf> {
    if !config.node.working_dir.trim().is_empty() {
        return Some(PathBuf::from(&config.node.working_dir));
    }

    dirs::home_dir().map(|path| path.join(".openclaw"))
}
