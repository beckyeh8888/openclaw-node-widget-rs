pub mod openclaw;
pub mod ollama;
pub mod openai_compat;
pub mod registry;

use std::fmt;

use tokio::sync::mpsc;

use crate::chat::ChatMessage;
use crate::gateway::ChatAttachment;

// ── Value Objects ───────────────────────────────────────────────────

/// Unique identifier for a plugin instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginId(pub String);

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Capabilities advertised by a plugin.
#[derive(Debug, Clone)]
pub struct PluginCapabilities {
    pub chat: bool,
    pub dashboard: bool,
    pub workflows: bool,
    pub logs: bool,
}

impl Default for PluginCapabilities {
    fn default() -> Self {
        Self {
            chat: true,
            dashboard: false,
            workflows: false,
            logs: false,
        }
    }
}

/// Connection status of a plugin.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Reconnecting,
    Error(String),
}

impl fmt::Display for ConnectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected => write!(f, "Connected"),
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Reconnecting => write!(f, "Reconnecting"),
            Self::Error(e) => write!(f, "Error: {e}"),
        }
    }
}

// ── Domain Events ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum PluginEvent {
    Connected(PluginId),
    Disconnected(PluginId, String),
    MessageReceived(PluginId, ChatMessage),
    StreamChunk { plugin_id: PluginId, text: String },
    StatusChanged(PluginId, ConnectionStatus),
    Error(PluginId, String),
}

// ── Errors ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PluginError(pub String);

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PluginError {}

// ── Commands sent to a plugin ───────────────────────────────────────

#[derive(Debug)]
pub enum PluginCommand {
    SendChat {
        message: String,
        session_key: Option<String>,
        attachments: Option<Vec<ChatAttachment>>,
    },
    ListSessions,
}

// ── Plugin Trait ────────────────────────────────────────────────────

/// Trait that every agent backend plugin must implement.
///
/// Plugins are `Send` so they can be held in structures shared across
/// the tao/tray event loop and tokio tasks.  All async work happens
/// internally (spawned on the tokio runtime); the trait surface is
/// synchronous.
pub trait AgentPlugin: Send {
    /// Unique identifier (e.g. `"openclaw-home"`).
    fn id(&self) -> &PluginId;

    /// Human-readable name shown in UI.
    fn name(&self) -> &str;

    /// Plugin type key (e.g. `"openclaw"`, `"ollama"`).
    fn plugin_type(&self) -> &str;

    /// Emoji or icon identifier.
    fn icon(&self) -> &str;

    /// Advertised capabilities.
    fn capabilities(&self) -> PluginCapabilities;

    /// Current connection status.
    fn status(&self) -> ConnectionStatus;

    // ── Lifecycle ───────────────────────────────────────────────────

    /// Start connecting (spawns async tasks internally).
    fn connect(&mut self) -> Result<(), PluginError>;

    /// Tear down connection.
    fn disconnect(&mut self) -> Result<(), PluginError>;

    // ── Chat ────────────────────────────────────────────────────────

    /// Send a chat message through this plugin.
    fn send_message(
        &self,
        message: &str,
        session_key: Option<String>,
        attachments: Option<Vec<ChatAttachment>>,
    ) -> Result<(), PluginError>;

    /// Request the list of available sessions/conversations.
    fn list_sessions(&self) -> Result<(), PluginError>;

    /// Returns a command sender that can be cloned and given to the
    /// chat UI so it can send commands without holding a mutable
    /// reference to the plugin.
    fn command_sender(&self) -> Option<mpsc::UnboundedSender<PluginCommand>>;
}
