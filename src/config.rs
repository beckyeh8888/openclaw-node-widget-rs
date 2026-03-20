use std::{collections::HashMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
    #[serde(default)]
    pub voice: VoiceConfig,
    pub tts: TtsConfig,
    #[serde(default)]
    pub update: UpdateConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    pub auto_check: bool,
    pub auto_download: bool,
    pub auto_restart: bool,
    pub check_interval_hours: u64,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            auto_check: true,
            auto_download: true,
            auto_restart: false,
            check_interval_hours: 6,
        }
    }
}

/// Pre-configured defaults for deploying to clients.
/// If a `[defaults]` section exists next to the exe, the wizard pre-fills these values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DefaultsConfig {
    pub gateway_host: Option<String>,
    pub gateway_port: Option<String>,
    pub gateway_token: Option<String>,
}

/// Agent discovery and switching configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    /// Which agent to select by default (e.g. "main").
    pub default: String,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            default: "main".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// New plugin-based config format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(rename = "type")]
    pub plugin_type: String,
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub poll_url: Option<String>,
    /// MCP transport type: "stdio" or "sse"
    #[serde(default)]
    pub transport: Option<String>,
    /// MCP stdio command (e.g. "npx")
    #[serde(default)]
    pub command: Option<String>,
    /// MCP stdio command args
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Optional system prompt sent as the first message to the LLM.
    #[serde(default)]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub enabled: bool,
    pub provider: String,
    pub openai_api_key: Option<String>,
    pub language: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "local".to_string(),
            openai_api_key: None,
            language: "auto".to_string(),
        }
    }
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
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub always_on_top: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsConfig {
    pub enabled: bool,
    pub auto_read: bool,
    pub voice: String,
    pub rate: f32,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_read: false,
            voice: "auto".to_string(),
            rate: 1.0,
        }
    }
}

/// Subset of settings exposed via the Settings UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    pub language: String,
    pub auto_start: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub always_on_top: bool,
}

fn default_theme() -> String {
    "auto".to_string()
}

// Default is derived via #[derive(Default)] on the struct.

// Default is derived via #[derive(Default)] on the struct.

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
            theme: "auto".to_string(),
            always_on_top: false,
        }
    }
}

// Default is derived via #[derive(Default)] on the struct.

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

    /// Returns the effective list of plugins.
    ///
    /// Priority: `[[plugins]]` → `[[connections]]` (auto-mapped to openclaw) → `[gateway]`.
    pub fn effective_plugins(&self) -> Vec<PluginConfig> {
        if !self.plugins.is_empty() {
            return self.plugins.clone();
        }

        // Migrate [[connections]] to plugin format
        let conns = self.effective_connections();
        conns
            .iter()
            .map(|c| PluginConfig {
                plugin_type: "openclaw".to_string(),
                name: c.name.clone(),
                url: Some(c.gateway_url.clone()),
                token: c.gateway_token.clone(),
                model: None,
                api_key: None,
                webhook_url: None,
                poll_url: None,
                transport: None,
                command: None,
                args: None,
                system_prompt: None,
            })
            .collect()
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

        // Auto-migrate: if [[connections]] exist but no [[plugins]], migrate
        if config.plugins.is_empty() && !config.connections.is_empty() {
            info!("migrating [[connections]] config to [[plugins]] format");
            config.plugins = config
                .connections
                .iter()
                .map(|c| PluginConfig {
                    plugin_type: "openclaw".to_string(),
                    name: c.name.clone(),
                    url: Some(c.gateway_url.clone()),
                    token: c.gateway_token.clone(),
                    model: None,
                    api_key: None,
                    webhook_url: None,
                    poll_url: None,
                    transport: None,
                    command: None,
                    args: None,
                    system_prompt: None,
                })
                .collect();
        }

        Ok(config)
    }

    /// Add or update a plugin by name. If a plugin with the same name exists,
    /// it is replaced; otherwise the new plugin is appended.
    pub fn upsert_plugin(&mut self, plugin: PluginConfig) {
        if let Some(existing) = self.plugins.iter_mut().find(|p| p.name == plugin.name) {
            *existing = plugin;
        } else {
            self.plugins.push(plugin);
        }
    }

    /// Remove a plugin by name. Returns true if a plugin was removed.
    pub fn remove_plugin(&mut self, name: &str) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|p| p.name != name);
        self.plugins.len() < before
    }

    /// Update general settings from a JSON-friendly struct.
    pub fn apply_general_settings(&mut self, general: &GeneralSettings) {
        self.widget.language = general.language.clone();
        self.startup.auto_start = general.auto_start;
        self.widget.theme = general.theme.clone();
        self.widget.always_on_top = general.always_on_top;
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

/// Check whether Node.js (npm) is available on this system.
pub fn detect_nodejs() -> bool {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new("node");
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

pub fn app_dir() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::Config("unable to resolve config directory".to_string()))?;
    Ok(config_dir.join("openclaw-node-widget"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("config.toml"))
}
