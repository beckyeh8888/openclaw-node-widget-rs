use std::time::Duration;

use serde::Deserialize;
use tracing::{debug, warn};

use crate::i18n::t;

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
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
    match notify_rust::Notification::new()
        .appname(t("app_name"))
        .summary(t("app_name"))
        .body(body)
        .show()
    {
        Ok(_) => debug!("update notification sent"),
        Err(e) => warn!("update notification failed: {e}"),
    }
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
