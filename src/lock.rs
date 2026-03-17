use std::{
    fs,
    path::{Path, PathBuf},
};

use sysinfo::System;

use crate::{
    config::app_dir,
    error::{AppError, Result},
};

pub struct LockGuard {
    path: PathBuf,
}

pub enum AcquireResult {
    Acquired(LockGuard),
    AlreadyRunning(i32),
}

pub fn lock_file_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("widget.lock"))
}

pub fn try_acquire_lock() -> Result<AcquireResult> {
    let path = lock_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    if path.exists() {
        let pid = read_pid(&path)?;
        if let Some(pid) = pid {
            if is_pid_running(pid) {
                return Ok(AcquireResult::AlreadyRunning(pid));
            }
        }
        fs::remove_file(&path)?;
    }

    let pid = std::process::id();
    fs::write(&path, pid.to_string())?;

    Ok(AcquireResult::Acquired(LockGuard { path }))
}

fn read_pid(path: &Path) -> Result<Option<i32>> {
    let content = fs::read_to_string(path)?;
    let value = content.trim();
    if value.is_empty() {
        return Ok(None);
    }

    let pid = value
        .parse::<i32>()
        .map_err(|_| AppError::Config(format!("invalid lockfile pid: {value}")))?;
    Ok(Some(pid))
}

fn is_pid_running(pid: i32) -> bool {
    let mut system = System::new_all();
    system.refresh_processes();

    system
        .processes()
        .values()
        .any(|process| process.pid().as_u32() as i32 == pid)
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
