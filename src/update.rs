#![allow(dead_code)]
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::config::UpdateConfig;
use crate::i18n::t;

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[cfg(windows)]
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[cfg(windows)]
#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Information about an available update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub download_url: String,
    pub release_notes: String,
    pub download_path: Option<PathBuf>,
}

impl UpdateInfo {
    pub fn new(version: String, download_url: String, release_notes: String) -> Self {
        Self {
            version,
            download_url,
            release_notes,
            download_path: None,
        }
    }
}

pub async fn check_for_updates() -> Option<(String, String)> {
    let url = "https://api.github.com/repos/beckyeh8888/openclaw-node-widget-rs/releases/latest";
    let client = reqwest::Client::builder()
        .user_agent(format!(
            "openclaw-node-widget/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;

    let response = client.get(url).send().await.ok()?;

    if !response.status().is_success() {
        debug!("GitHub API returned status: {}", response.status());
        return None;
    }

    let release: GitHubRelease = response.json().await.ok()?;
    let remote_version = release.tag_name.trim_start_matches('v');
    let current = env!("CARGO_PKG_VERSION");

    if version_is_newer(remote_version, current) {
        Some((release.tag_name, release.html_url))
    } else {
        None
    }
}

/// Check for updates and return structured UpdateInfo if a newer version is available.
pub async fn auto_update_check(current_version: &str) -> Option<UpdateInfo> {
    let url = "https://api.github.com/repos/beckyeh8888/openclaw-node-widget-rs/releases/latest";
    let client = reqwest::Client::builder()
        .user_agent(format!("openclaw-node-widget/{current_version}"))
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;

    let response = client.get(url).send().await.ok()?;

    if !response.status().is_success() {
        debug!("GitHub API returned status: {}", response.status());
        return None;
    }

    let release: GitHubRelease = response.json().await.ok()?;
    let remote_version = release.tag_name.trim_start_matches('v');

    if version_is_newer(remote_version, current_version) {
        Some(UpdateInfo::new(
            release.tag_name.clone(),
            release.html_url,
            release.body.unwrap_or_default(),
        ))
    } else {
        None
    }
}

/// Download the update binary to a temp directory. Returns the path to the
/// downloaded file on success.
pub async fn download_update(info: &UpdateInfo) -> Result<PathBuf, String> {
    let tag = &info.version;
    let download_url = format!(
        "https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/download/{tag}/openclaw-node-widget-{os}-{arch}",
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
    );

    let client = reqwest::Client::builder()
        .user_agent(format!(
            "openclaw-node-widget/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;

    tracing::info!("downloading update from {download_url}");

    let response = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "download returned status {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("failed to read download: {e}"))?;

    let temp_dir = std::env::temp_dir().join("openclaw-update");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp dir: {e}"))?;

    let binary_name = if cfg!(windows) {
        format!("openclaw-node-widget-{tag}.exe")
    } else {
        format!("openclaw-node-widget-{tag}")
    };
    let dest = temp_dir.join(binary_name);
    std::fs::write(&dest, &bytes)
        .map_err(|e| format!("failed to write update binary: {e}"))?;

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }

    tracing::info!("update downloaded to {}", dest.display());
    Ok(dest)
}

/// Apply a downloaded update by replacing the current binary and restarting.
pub fn apply_update(info: &UpdateInfo) -> Result<(), String> {
    let download_path = info
        .download_path
        .as_ref()
        .ok_or_else(|| "no download path set — download the update first".to_string())?;

    if !download_path.exists() {
        return Err(format!(
            "downloaded binary not found: {}",
            download_path.display()
        ));
    }

    let current_exe =
        std::env::current_exe().map_err(|e| format!("cannot resolve current exe: {e}"))?;

    // Back up current binary
    let bak_path = current_exe.with_extension("bak");
    if current_exe.exists() {
        let _ = std::fs::remove_file(&bak_path);
        std::fs::rename(&current_exe, &bak_path)
            .map_err(|e| format!("failed to backup current binary: {e}"))?;
    }

    // Copy new binary into place
    std::fs::copy(download_path, &current_exe)
        .map_err(|e| format!("failed to install new binary: {e}"))?;

    // Set executable permission on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755));
    }

    tracing::info!("update applied, restarting...");

    // Restart: spawn new process and exit
    let _ = std::process::Command::new(&current_exe).spawn();
    std::process::exit(0);
}

/// Spawn a periodic update check task that respects UpdateConfig settings.
pub fn spawn_periodic_check_with_config(update_config: &UpdateConfig) {
    if !update_config.auto_check {
        debug!("auto update check disabled");
        return;
    }

    let interval_hours = update_config.check_interval_hours.max(1);
    let auto_download = update_config.auto_download;

    tokio::spawn(async move {
        // Check on startup after a short delay
        tokio::time::sleep(Duration::from_secs(30)).await;
        do_check_v2(auto_download).await;

        // Then at configured interval
        let mut interval =
            tokio::time::interval(Duration::from_secs(interval_hours * 3600));
        loop {
            interval.tick().await;
            do_check_v2(auto_download).await;
        }
    });
}

pub fn spawn_periodic_check() {
    tokio::spawn(async {
        // Check on startup after a short delay
        tokio::time::sleep(Duration::from_secs(30)).await;
        do_check().await;

        // Then every 6 hours
        let mut interval = tokio::time::interval(Duration::from_secs(6 * 3600));
        loop {
            interval.tick().await;
            do_check().await;
        }
    });
}

async fn do_check() {
    match check_for_updates().await {
        Some((version, url)) => {
            let body = format!("{} {version}\n{url}", t("notif_update_available"));
            notify_update(&body);
        }
        None => {
            debug!("no updates available");
        }
    }
}

async fn do_check_v2(auto_download: bool) {
    let current = env!("CARGO_PKG_VERSION");
    match auto_update_check(current).await {
        Some(mut info) => {
            if auto_download {
                match download_update(&info).await {
                    Ok(path) => {
                        info.download_path = Some(path);
                        let msg = format!(
                            "Update {} ready — restart to apply",
                            info.version
                        );
                        notify_update(&msg);
                    }
                    Err(e) => {
                        tracing::warn!("auto-download failed: {e}");
                        let body = format!(
                            "{} {}\n{}",
                            t("notif_update_available"),
                            info.version,
                            info.download_url
                        );
                        notify_update(&body);
                    }
                }
            } else {
                let body = format!(
                    "{} {}\n{}",
                    t("notif_update_available"),
                    info.version,
                    info.download_url
                );
                notify_update(&body);
            }
        }
        None => {
            debug!("no updates available");
        }
    }
}

fn notify_update(body: &str) {
    crate::tray::send_notification_public(body);
}

/// Download and install an update from the given release tag.
///
/// On Windows: downloads the zip asset, extracts the exe, renames the current
/// exe to .bak, and copies the new exe into place. Shows a notification asking
/// the user to restart.
///
/// On other platforms: opens the release page in the browser.
pub async fn download_and_install(tag: &str) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let url = format!(
            "https://github.com/beckyeh8888/openclaw-node-widget-rs/releases/tag/{tag}"
        );
        debug!("opening release page: {url}");
        let _ = open::that(&url);
        Ok(())
    }

    #[cfg(windows)]
    {
        download_and_install_windows(tag).await
    }
}

#[cfg(windows)]
async fn download_and_install_windows(tag: &str) -> Result<(), String> {
    use std::io::Read;

    let api_url = format!(
        "https://api.github.com/repos/beckyeh8888/openclaw-node-widget-rs/releases/tags/{tag}"
    );

    let client = reqwest::Client::builder()
        .user_agent(format!(
            "openclaw-node-widget/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;

    // Fetch release metadata to find the Windows zip asset
    let release: GitHubRelease = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| format!("failed to fetch release: {e}"))?
        .json()
        .await
        .map_err(|e| format!("failed to parse release: {e}"))?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name.contains("windows") && a.name.ends_with(".zip"))
        .ok_or_else(|| "no Windows zip asset found in release".to_string())?;

    tracing::info!("downloading update: {}", asset.browser_download_url);

    // Download the zip
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("failed to read download: {e}"))?;

    // Extract exe from zip
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("failed to open zip: {e}"))?;

    let mut exe_data = Vec::new();
    let mut found = false;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("zip entry error: {e}"))?;
        if file.name().ends_with(".exe") {
            file.read_to_end(&mut exe_data)
                .map_err(|e| format!("failed to read exe from zip: {e}"))?;
            found = true;
            break;
        }
    }
    if !found {
        return Err("no .exe found in zip archive".to_string());
    }

    // Install to the proper install directory
    let install_dir = crate::install::windows_install_dir()
        .map_err(|e| format!("cannot resolve install dir: {e}"))?;
    let install_exe = crate::install::windows_install_exe()
        .map_err(|e| format!("cannot resolve install exe: {e}"))?;

    std::fs::create_dir_all(&install_dir)
        .map_err(|e| format!("failed to create install dir: {e}"))?;

    // Write the new exe to install path (backup old if present)
    let bak_path = install_exe.with_extension("exe.bak");
    if install_exe.exists() {
        let _ = std::fs::remove_file(&bak_path);
        let _ = std::fs::rename(&install_exe, &bak_path);
    }

    std::fs::write(&install_exe, &exe_data)
        .map_err(|e| format!("failed to write new exe: {e}"))?;

    tracing::info!("update installed to {}", install_exe.display());

    // Update autostart registry and Start Menu shortcut to point to install path
    if let Err(e) = crate::install::perform_install() {
        tracing::warn!("post-install setup: {e}");
    }

    // Auto-restart: spawn the installed exe and exit current process
    tracing::info!("auto-restarting widget from install path...");
    let _ = std::process::Command::new(&install_exe).spawn();
    std::process::exit(0);
}

pub fn version_is_newer(remote: &str, current: &str) -> bool {
    let parse = |v: &str| -> (u32, u32, u32) {
        let parts: Vec<u32> = v.split('.').filter_map(|p| p.parse().ok()).collect();
        (
            parts.first().copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };
    parse(remote) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── version_is_newer ───────────────────────────────────────

    #[test]
    fn given_newer_patch_version_then_is_newer() {
        assert!(version_is_newer("0.9.1", "0.9.0"));
    }

    #[test]
    fn given_newer_minor_version_then_is_newer() {
        assert!(version_is_newer("0.10.0", "0.9.5"));
    }

    #[test]
    fn given_newer_major_version_then_is_newer() {
        assert!(version_is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn given_same_version_then_not_newer() {
        assert!(!version_is_newer("0.9.0", "0.9.0"));
    }

    #[test]
    fn given_older_version_then_not_newer() {
        assert!(!version_is_newer("0.8.0", "0.9.0"));
    }

    #[test]
    fn given_version_with_v_prefix_stripped_then_is_newer() {
        // version_is_newer expects the prefix already stripped
        let remote = "v1.0.0".trim_start_matches('v');
        assert!(version_is_newer(remote, "0.9.0"));
    }

    // ── UpdateInfo ─────────────────────────────────────────────

    #[test]
    fn given_new_update_info_then_fields_are_correct() {
        let info = UpdateInfo::new(
            "v0.9.1".to_string(),
            "https://example.com/release".to_string(),
            "Bug fixes and improvements".to_string(),
        );
        assert_eq!(info.version, "v0.9.1");
        assert_eq!(info.download_url, "https://example.com/release");
        assert_eq!(info.release_notes, "Bug fixes and improvements");
        assert!(info.download_path.is_none());
    }

    #[test]
    fn given_update_info_with_download_path_then_path_is_set() {
        let mut info = UpdateInfo::new(
            "v0.9.1".to_string(),
            "https://example.com".to_string(),
            String::new(),
        );
        info.download_path = Some(PathBuf::from("/tmp/update-binary"));
        assert_eq!(
            info.download_path.as_ref().unwrap().to_str().unwrap(),
            "/tmp/update-binary"
        );
    }

    // ── UpdateConfig defaults ──────────────────────────────────

    #[test]
    fn given_default_update_config_then_auto_check_enabled() {
        let config = UpdateConfig::default();
        assert!(config.auto_check);
        assert!(config.auto_download);
        assert!(!config.auto_restart);
        assert_eq!(config.check_interval_hours, 6);
    }

    #[test]
    fn given_auto_check_disabled_then_no_checks() {
        let config = UpdateConfig {
            auto_check: false,
            ..UpdateConfig::default()
        };
        assert!(!config.auto_check);
    }

    #[test]
    fn given_custom_interval_then_interval_is_used() {
        let config = UpdateConfig {
            check_interval_hours: 12,
            ..UpdateConfig::default()
        };
        assert_eq!(config.check_interval_hours, 12);
    }

    // ── apply_update error cases ───────────────────────────────

    #[test]
    fn given_update_info_without_download_path_then_apply_fails() {
        let info = UpdateInfo::new(
            "v1.0.0".to_string(),
            "https://example.com".to_string(),
            String::new(),
        );
        let result = apply_update(&info);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("no download path set"));
    }

    #[test]
    fn given_update_info_with_nonexistent_path_then_apply_fails() {
        let mut info = UpdateInfo::new(
            "v1.0.0".to_string(),
            "https://example.com".to_string(),
            String::new(),
        );
        info.download_path = Some(PathBuf::from("/nonexistent/path/binary"));
        let result = apply_update(&info);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("downloaded binary not found"));
    }

    // ── Serialization ──────────────────────────────────────────

    #[test]
    fn given_update_info_then_serializes_to_json() {
        let info = UpdateInfo::new(
            "v0.9.1".to_string(),
            "https://example.com/release".to_string(),
            "Notes".to_string(),
        );
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("v0.9.1"));
        assert!(json.contains("Notes"));
    }

    #[test]
    fn given_json_then_deserializes_to_update_info() {
        let json = r#"{"version":"v1.0.0","download_url":"https://example.com","release_notes":"test","download_path":null}"#;
        let info: UpdateInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.version, "v1.0.0");
        assert!(info.download_path.is_none());
    }

    // ── Config integration ─────────────────────────────────────

    #[test]
    fn given_config_with_update_section_then_deserialized() {
        let toml_str = r#"
auto_check = false
auto_download = false
auto_restart = true
check_interval_hours = 12
"#;
        let config: UpdateConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.auto_check);
        assert!(!config.auto_download);
        assert!(config.auto_restart);
        assert_eq!(config.check_interval_hours, 12);
    }

    #[test]
    fn given_full_config_toml_with_update_section_then_parsed() {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            #[serde(default)]
            update: UpdateConfig,
        }
        let toml_str = r#"
[update]
auto_check = false
check_interval_hours = 24
"#;
        let w: Wrapper = toml::from_str(toml_str).unwrap();
        assert!(!w.update.auto_check);
        assert_eq!(w.update.check_interval_hours, 24);
        // Defaults for unspecified fields
        assert!(w.update.auto_download);
        assert!(!w.update.auto_restart);
    }
}
