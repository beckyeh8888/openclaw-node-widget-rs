use std::time::Duration;

use serde::Deserialize;
use tracing::debug;

use crate::i18n::t;

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
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
        return Ok(());
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

    // Replace current exe: rename current to .bak, write new exe
    let current_exe =
        std::env::current_exe().map_err(|e| format!("cannot resolve current exe: {e}"))?;
    let bak_path = current_exe.with_extension("exe.bak");

    // Remove old backup if it exists
    let _ = std::fs::remove_file(&bak_path);

    std::fs::rename(&current_exe, &bak_path)
        .map_err(|e| format!("failed to rename current exe to .bak: {e}"))?;

    std::fs::write(&current_exe, &exe_data)
        .map_err(|e| format!("failed to write new exe: {e}"))?;

    tracing::info!("update installed to {}", current_exe.display());
    Ok(())
}

fn version_is_newer(remote: &str, current: &str) -> bool {
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
