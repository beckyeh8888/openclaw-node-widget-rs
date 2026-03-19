use tokio::sync::mpsc;

use super::{
    AgentPlugin, ConnectionStatus, PluginCapabilities, PluginCommand, PluginError, PluginId,
};
use crate::config::PluginConfig;
use crate::gateway::ChatAttachment;

/// OpenAI-compatible API plugin stub.
///
/// Works with any OpenAI-compatible endpoint: OpenAI, Ollama
/// (via its OpenAI-compat layer), LM Studio, vLLM, etc.
///
/// This is a placeholder implementation.
pub struct OpenAICompatPlugin {
    id: PluginId,
    plugin_name: String,
    url: String,
    model: String,
    #[allow(dead_code)]
    api_key: Option<String>,
    status: ConnectionStatus,
    cmd_tx: Option<mpsc::UnboundedSender<PluginCommand>>,
}

impl OpenAICompatPlugin {
    pub fn new(config: &PluginConfig) -> Self {
        let id_str = format!("openai-{}", slug(&config.name));
        Self {
            id: PluginId(id_str),
            plugin_name: config.name.clone(),
            url: config
                .url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: config
                .model
                .clone()
                .unwrap_or_else(|| "gpt-4o".to_string()),
            api_key: config.api_key.clone(),
            status: ConnectionStatus::Disconnected,
            cmd_tx: None,
        }
    }
}

impl AgentPlugin for OpenAICompatPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.plugin_name
    }

    fn plugin_type(&self) -> &str {
        "openai-compatible"
    }

    fn icon(&self) -> &str {
        "🤖"
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
            return Err(PluginError("API URL not configured".to_string()));
        }
        tracing::info!(
            plugin = %self.id,
            url = %self.url,
            model = %self.model,
            "openai-compatible plugin connect (stub)"
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
            "openai-compatible send_message (stub — not implemented)"
        );
        Err(PluginError(
            "openai-compatible chat not yet implemented".to_string(),
        ))
    }

    fn list_sessions(&self) -> Result<(), PluginError> {
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
