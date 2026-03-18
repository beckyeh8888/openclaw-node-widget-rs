use std::process::{Command, Stdio};

use serde_json::Value;
use tracing::debug;

/// A discovered Tailscale peer.
#[derive(Debug, Clone)]
pub struct TailscalePeer {
    pub hostname: String,
    pub ip: String,
}

/// Current Tailscale status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailscaleStatus {
    Connected,
    Disconnected,
    NotInstalled,
}

/// Try to get the Tailscale binary name/path for the current platform.
fn tailscale_cmd() -> Vec<&'static str> {
    #[cfg(windows)]
    {
        // Try the standard install path first, then bare command
        vec![
            r"C:\Program Files\Tailscale\tailscale.exe",
            "tailscale.exe",
            "tailscale",
        ]
    }
    #[cfg(not(windows))]
    {
        vec!["tailscale"]
    }
}

/// Run `tailscale status --json` and return parsed JSON, or None if unavailable.
fn run_tailscale_status_json() -> Option<Value> {
    for cmd in tailscale_cmd() {
        let result = Command::new(cmd)
            .args(["status", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();

        match result {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                match serde_json::from_str::<Value>(&text) {
                    Ok(val) => return Some(val),
                    Err(e) => {
                        debug!("tailscale JSON parse error: {e}");
                        continue;
                    }
                }
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    None
}

/// Run `tailscale status` (non-JSON) to check if Tailscale is running.
fn run_tailscale_status_simple() -> Option<bool> {
    for cmd in tailscale_cmd() {
        let result = Command::new(cmd)
            .args(["status"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();

        match result {
            Ok(output) => return Some(output.status.success()),
            Err(_) => continue,
        }
    }
    None
}

/// Detect Tailscale peers from `tailscale status --json`.
/// Returns a list of peers with their hostnames and Tailscale IPs.
pub fn detect_peers() -> Vec<TailscalePeer> {
    let Some(json) = run_tailscale_status_json() else {
        return Vec::new();
    };

    let mut peers = Vec::new();

    // The "Peer" field is a map of public-key -> peer info
    if let Some(peer_map) = json.get("Peer").and_then(Value::as_object) {
        for (_key, peer) in peer_map {
            let hostname = peer
                .get("HostName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            // TailscaleIPs is an array of IP strings
            let ip = peer
                .get("TailscaleIPs")
                .and_then(Value::as_array)
                .and_then(|ips| ips.first())
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            if !hostname.is_empty() && !ip.is_empty() {
                peers.push(TailscalePeer { hostname, ip });
            }
        }
    }

    // Sort by hostname for consistent display
    peers.sort_by(|a, b| a.hostname.cmp(&b.hostname));
    peers
}

/// Check if Tailscale is installed and connected.
pub fn check_status() -> TailscaleStatus {
    match run_tailscale_status_simple() {
        Some(true) => TailscaleStatus::Connected,
        Some(false) => TailscaleStatus::Disconnected,
        None => TailscaleStatus::NotInstalled,
    }
}

/// Returns true if the given IP looks like a Tailscale IP (100.x.x.x CGNAT range).
pub fn is_tailscale_ip(ip: &str) -> bool {
    // Tailscale uses 100.64.0.0/10 (CGNAT range)
    // In practice, IPs are 100.x.x.x where x >= 64
    let stripped = ip
        .strip_prefix("ws://")
        .or_else(|| ip.strip_prefix("wss://"))
        .unwrap_or(ip);
    // Strip port if present
    let host = stripped.split(':').next().unwrap_or(stripped);
    host.starts_with("100.")
}
