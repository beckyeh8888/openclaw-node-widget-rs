use std::{collections::HashMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub gateway: GatewayConfig,
    pub node: NodeConfig,
    pub widget: WidgetConfig,
    pub startup: StartupConfig,
    pub appearance: AppearanceConfig,
    pub log: LogConfig,
    #[serde(default)]
    pub connections: Vec<ConnectionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub name: String,
    pub gateway_url: String,
    #[serde(default)]
    pub gateway_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeConfig {
    pub command: String,
    pub working_dir: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetConfig {
    pub check_interval_secs: u64,
    pub auto_restart: bool,
    pub restart_threshold: u32,
    pub restart_cooldown_secs: u64,
    pub max_restart_attempts: u32,
    pub crash_loop_secs: u64,
    pub notifications: bool,
    pub notification_sound: bool,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupConfig {
    pub auto_start: bool,
    pub xdg_desktop_path: String,
    pub launchd_plist_path: String,
    pub registry_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub online_icon: String,
    pub offline_icon: String,
    pub unknown_icon: String,
    pub tooltip_format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: String,
    pub file: String,
    pub syslog: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gateway: GatewayConfig::default(),
            node: NodeConfig::default(),
            widget: WidgetConfig::default(),
            startup: StartupConfig::default(),
            appearance: AppearanceConfig::default(),
            log: LogConfig::default(),
            connections: Vec::new(),
        }
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            url: None,
            token: None,
        }
    }
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            command: "openclaw node run".to_string(),
            working_dir: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
        }
    }
}

impl Default for WidgetConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: 15,
            auto_restart: true,
            restart_threshold: 3,
            restart_cooldown_secs: 120,
            max_restart_attempts: 5,
            crash_loop_secs: 300,
            notifications: true,
            notification_sound: true,
            language: "auto".to_string(),
        }
    }
}

impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            xdg_desktop_path: String::new(),
            launchd_plist_path: String::new(),
            registry_key: String::new(),
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            online_icon: String::new(),
            offline_icon: String::new(),
            unknown_icon: String::new(),
            tooltip_format: "OpenClaw Node: {status}".to_string(),
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            file: String::new(),
            syslog: false,
        }
    }
}

impl Config {
    /// Returns the effective list of connections.
    /// If `[[connections]]` is populated, use those.
    /// Otherwise, if old-style `[gateway]` has a URL, treat it as a single "Default" connection.
    pub fn effective_connections(&self) -> Vec<ConnectionConfig> {
        if !self.connections.is_empty() {
            return self.connections.clone();
        }

        // Backward compat: convert old [gateway] section
        if let Some(url) = self.gateway.url.as_ref().filter(|u| !u.trim().is_empty()) {
            vec![ConnectionConfig {
                name: "Default".to_string(),
                gateway_url: url.clone(),
                gateway_token: self.gateway.token.clone(),
            }]
        } else {
            Vec::new()
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Validate old-style gateway if no connections defined
        if self.connections.is_empty() {
            if let Some(url) = &self.gateway.url {
                if !url.is_empty() && !url.starts_with("ws://") && !url.starts_with("wss://") {
                    errors.push(format!("Gateway URL must start with ws:// or wss://, got: {url}"));
                }
            }

            if let Some(token) = &self.gateway.token {
                if token.trim().is_empty() {
                    errors.push("Gateway token is empty".to_string());
                }
            }
        }

        // Validate each connection
        for (i, conn) in self.connections.iter().enumerate() {
            let label = if conn.name.is_empty() {
                format!("Connection #{}", i + 1)
            } else {
                conn.name.clone()
            };
            if conn.gateway_url.is_empty() {
                errors.push(format!("{label}: gateway_url is empty"));
            } else if !conn.gateway_url.starts_with("ws://") && !conn.gateway_url.starts_with("wss://") {
                errors.push(format!("{label}: gateway_url must start with ws:// or wss://"));
            }
            if let Some(token) = &conn.gateway_token {
                if token.trim().is_empty() {
                    errors.push(format!("{label}: gateway_token is empty"));
                }
            }
        }

        if !self.node.command.is_empty() {
            let cmd = self.node.command.split_whitespace().next().unwrap_or("");
            if !cmd.is_empty() && !cmd.contains('/') && !cmd.contains('\\') {
                // Only check if it's a path, not a bare command name (which relies on PATH)
            } else if !cmd.is_empty() && !std::path::Path::new(cmd).exists() {
                errors.push(format!("Node command not found: {cmd}"));
            }
        }

        if !self.node.working_dir.is_empty() && !std::path::Path::new(&self.node.working_dir).exists() {
            errors.push(format!("Node working directory does not exist: {}", self.node.working_dir));
        }

        errors
    }

    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)?;
        let mut config: Config = toml::from_str(&content).map_err(|e| AppError::Config(e.to_string()))?;

        // Auto-migrate: if old [gateway] exists but no [[connections]], migrate
        if config.connections.is_empty() {
            if let Some(url) = config.gateway.url.as_ref().filter(|u| !u.trim().is_empty()) {
                info!("migrating old [gateway] config to [[connections]] format");
                config.connections.push(ConnectionConfig {
                    name: "Default".to_string(),
                    gateway_url: url.clone(),
                    gateway_token: config.gateway.token.clone(),
                });
                // Clear old gateway section
                config.gateway.url = None;
                config.gateway.token = None;
                // Save migrated config
                if let Err(e) = config.save() {
                    tracing::warn!("failed to save migrated config: {e}");
                }
            }
        }

        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| AppError::Config(e.to_string()))?;
        fs::write(path, content)?;
        Ok(())
    }
}

pub fn app_dir() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::Config("unable to resolve config directory".to_string()))?;
    Ok(config_dir.join("openclaw-node-widget"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("config.toml"))
}
