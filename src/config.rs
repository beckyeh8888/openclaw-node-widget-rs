use std::{collections::HashMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    pub url: Option<String>,
    pub token: Option<String>,
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
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)?;
        toml::from_str(&content).map_err(|e| AppError::Config(e.to_string()))
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
