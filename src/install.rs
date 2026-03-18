#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use tracing::info;

#[cfg(windows)]
use crate::error::{AppError, Result};

/// Returns the canonical install directory for Windows:
/// `%LOCALAPPDATA%\OpenClaw Node Widget\`
#[cfg(windows)]
pub fn windows_install_dir() -> Result<PathBuf> {
    let local_app_data = dirs::data_local_dir()
        .ok_or_else(|| AppError::Config("cannot resolve %LOCALAPPDATA%".to_string()))?;
    Ok(local_app_data.join("OpenClaw Node Widget"))
}

/// Returns the expected exe path inside the install directory.
#[cfg(windows)]
pub fn windows_install_exe() -> Result<PathBuf> {
    Ok(windows_install_dir()?.join("openclaw-node-widget.exe"))
}

/// Returns true if the current exe is running from the install directory.
#[cfg(windows)]
pub fn is_running_from_install_dir() -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    let Ok(install_dir) = windows_install_dir() else {
        return false;
    };
    // Normalize both paths for comparison
    let current = current.to_string_lossy().to_lowercase();
    let install_dir = install_dir.to_string_lossy().to_lowercase();
    current.starts_with(&install_dir)
}

/// Perform a full install to the system install directory:
/// - Copy exe to `%LOCALAPPDATA%\OpenClaw Node Widget\`
/// - Set autostart registry entry pointing to the installed exe
/// - Create Start Menu shortcut
/// - Show notification
#[cfg(windows)]
pub fn perform_install() -> Result<()> {
    use std::fs;

    let install_dir = windows_install_dir()?;
    let install_exe = windows_install_exe()?;

    // Create install directory
    fs::create_dir_all(&install_dir)?;

    // Copy current exe to install path
    let current_exe =
        std::env::current_exe().map_err(|e| AppError::Process(format!("cannot resolve exe: {e}")))?;
    if current_exe != install_exe {
        fs::copy(&current_exe, &install_exe)
            .map_err(|e| AppError::Process(format!("failed to copy exe: {e}")))?;
        info!("installed exe to {}", install_exe.display());
    }

    // Set autostart registry entry pointing to installed path
    set_autostart_to_install_path(&install_exe)?;

    // Create Start Menu shortcut
    if let Err(e) = create_start_menu_shortcut(&install_exe) {
        tracing::warn!("failed to create Start Menu shortcut: {e}");
    }

    info!("installation complete");
    Ok(())
}

/// Update the autostart registry entry to point to the given exe path.
#[cfg(windows)]
fn set_autostart_to_install_path(exe_path: &std::path::Path) -> Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    let (run_key, _) = hkcu
        .create_subkey(path)
        .map_err(|e| AppError::Process(format!("registry open failed: {e}")))?;

    let exe_str = exe_path.to_string_lossy().to_string();
    run_key
        .set_value("OpenClawNodeWidget", &exe_str)
        .map_err(|e| AppError::Process(format!("registry write failed: {e}")))?;

    info!("autostart registry set to {exe_str}");
    Ok(())
}

/// Create a Start Menu shortcut using a PowerShell COM object invocation.
/// Creates at `%APPDATA%\Microsoft\Windows\Start Menu\Programs\OpenClaw Node Widget.lnk`
#[cfg(windows)]
fn create_start_menu_shortcut(exe_path: &std::path::Path) -> Result<()> {
    let start_menu = dirs::data_dir()
        .ok_or_else(|| AppError::Config("cannot resolve %APPDATA%".to_string()))?
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs");

    std::fs::create_dir_all(&start_menu)?;
    let lnk_path = start_menu.join("OpenClaw Node Widget.lnk");
    let exe_str = exe_path.to_string_lossy().to_string();
    let lnk_str = lnk_path.to_string_lossy().to_string();

    // Use PowerShell to create .lnk shortcut via COM
    let ps_script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{}'); \
         $s.TargetPath = '{}'; \
         $s.Description = 'OpenClaw Node Widget'; \
         $s.Save()",
        lnk_str.replace('\'', "''"),
        exe_str.replace('\'', "''"),
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .output()
        .map_err(|e| AppError::Process(format!("powershell failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Process(format!(
            "shortcut creation failed: {stderr}"
        )));
    }

    info!("Start Menu shortcut created at {}", lnk_path.display());
    Ok(())
}

/// Remove the Start Menu shortcut.
#[cfg(windows)]
pub fn remove_start_menu_shortcut() {
    if let Some(start_menu) = dirs::data_dir() {
        let lnk_path = start_menu
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("OpenClaw Node Widget.lnk");
        let _ = std::fs::remove_file(lnk_path);
    }
}

/// Remove the installed exe directory.
#[cfg(windows)]
pub fn remove_install_dir() {
    if let Ok(dir) = windows_install_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Launch the installed exe and exit the current process.
#[cfg(windows)]
pub fn launch_installed_and_exit() -> ! {
    if let Ok(exe) = windows_install_exe() {
        let _ = std::process::Command::new(&exe).spawn();
    }
    std::process::exit(0);
}
