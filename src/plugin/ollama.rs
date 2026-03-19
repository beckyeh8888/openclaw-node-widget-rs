use tokio::sync::mpsc;

use super::{
    AgentPlugin, ConnectionStatus, PluginCapabilities, PluginCommand, PluginError, PluginId,
};
use crate::config::PluginConfig;
use crate::gateway::ChatAttachment;

/// Ollama plugin stub — connects to a local Ollama instance.
///
/// This is a placeholder implementation.  The `connect` method validates
/// the configured URL but does not yet establish a real connection.
pub struct OllamaPlugin {
    id: PluginId,
    plugin_name: String,
    url: String,
    model: String,
    status: ConnectionStatus,
    cmd_tx: Option<mpsc::UnboundedSender<PluginCommand>>,
}

impl OllamaPlugin {
    pub fn new(config: &PluginConfig) -> Self {
        let id_str = format!("ollama-{}", slug(&config.name));
        Self {
            id: PluginId(id_str),
            plugin_name: config.name.clone(),
            url: config
                .url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            model: config
                .model
                .clone()
                .unwrap_or_else(|| "llama3.3".to_string()),
            status: ConnectionStatus::Disconnected,
            cmd_tx: None,
        }
    }
}

impl AgentPlugin for OllamaPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.plugin_name
    }

    fn plugin_type(&self) -> &str {
        "ollama"
    }

    fn icon(&self) -> &str {
        "🦙"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities {
            chat: true,
            dashboard: false,
            workflows: false,
            logs: false,
        }
    }

    fn status(&self) -> ConnectionStatus {
        self.status.clone()
    }

    fn connect(&mut self) -> Result<(), PluginError> {
        if self.url.is_empty() {
            return Err(PluginError("ollama URL not configured".to_string()));
        }
        // Stub: mark as connected without actually connecting
        tracing::info!(
            plugin = %self.id,
            url = %self.url,
            model = %self.model,
            "ollama plugin connect (stub)"
        );
        self.status = ConnectionStatus::Connected;

        let (tx, _rx) = mpsc::unbounded_channel();
        self.cmd_tx = Some(tx);

        Ok(())
    }

    fn disconnect(&mut self) -> Result<(), PluginError> {
        self.status = ConnectionStatus::Disconnected;
        self.cmd_tx = None;
        Ok(())
    }

    fn send_message(
        &self,
        message: &str,
        _session_key: Option<String>,
        _attachments: Option<Vec<ChatAttachment>>,
    ) -> Result<(), PluginError> {
        tracing::info!(
            plugin = %self.id,
            model = %self.model,
            msg_len = message.len(),
            "ollama send_message (stub — not implemented)"
        );
        Err(PluginError("ollama chat not yet implemented".to_string()))
    }

    fn list_sessions(&self) -> Result<(), PluginError> {
        // Ollama doesn't have sessions
        Ok(())
    }

    fn command_sender(&self) -> Option<mpsc::UnboundedSender<PluginCommand>> {
        self.cmd_tx.clone()
    }
}

fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
