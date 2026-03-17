use std::env;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::{fs, path::PathBuf};

use crate::{
    config::Config,
    error::{AppError, Result},
};

const AUTOSTART_NAME: &str = "OpenClawNodeWidget";

pub fn set_autostart(enabled: bool) -> Result<()> {
    #[cfg(windows)]
    {
        return windows_set_autostart(enabled);
    }

    #[cfg(target_os = "macos")]
    {
        return macos_set_autostart(enabled);
    }

    #[cfg(target_os = "linux")]
    {
        return linux_set_autostart(enabled);
    }

    #[allow(unreachable_code)]
    Err(AppError::Process(
        "autostart is not supported on this platform".to_string(),
    ))
}

pub fn is_autostart_enabled() -> bool {
    #[cfg(windows)]
    {
        return windows_is_autostart_enabled();
    }

    #[cfg(target_os = "macos")]
    {
        return macos_plist_path().exists();
    }

    #[cfg(target_os = "linux")]
    {
        return linux_desktop_path().exists();
    }

    #[allow(unreachable_code)]
    false
}

pub fn effective_autostart(config: &Config) -> bool {
    config.startup.auto_start || is_autostart_enabled()
}

#[cfg(windows)]
fn windows_set_autostart(enabled: bool) -> Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    let (run_key, _) = hkcu
        .create_subkey(path)
        .map_err(|e| AppError::Process(format!("registry open failed: {e}")))?;

    if enabled {
        let exe = current_exe_string()?;
        run_key
            .set_value(AUTOSTART_NAME, &exe)
            .map_err(|e| AppError::Process(format!("registry write failed: {e}")))?;
    } else {
        let _ = run_key.delete_value(AUTOSTART_NAME);
    }

    Ok(())
}

#[cfg(windows)]
fn windows_is_autostart_enabled() -> bool {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_READ},
        RegKey,
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        KEY_READ,
    )
    .ok()
    .and_then(|key| key.get_value::<String, _>(AUTOSTART_NAME).ok())
    .is_some()
}

#[cfg(target_os = "macos")]
fn macos_set_autostart(enabled: bool) -> Result<()> {
    let path = macos_plist_path();

    if enabled {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let exe = current_exe_string()?;
        let escaped = xml_escape(&exe);
        let content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.openclaw.node-widget</string>
    <key>ProgramArguments</key>
    <array>
        <string>{escaped}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
        );
        fs::write(path, content)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_set_autostart(enabled: bool) -> Result<()> {
    let path = linux_desktop_path();

    if enabled {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let exe = current_exe_string()?;
        let content = format!(
            "[Desktop Entry]\nType=Application\nVersion=1.0\nName=OpenClaw Node Widget\nExec={exe}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
        );
        fs::write(path, content)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join("Library")
        .join("LaunchAgents")
        .join("ai.openclaw.node-widget.plist")
}

#[cfg(target_os = "linux")]
fn linux_desktop_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".config")
        .join("autostart")
        .join("openclaw-node-widget.desktop")
}

fn current_exe_string() -> Result<String> {
    env::current_exe()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|e| AppError::Process(format!("unable to resolve current exe: {e}")))
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
