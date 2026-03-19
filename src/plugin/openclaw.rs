use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{
    AgentPlugin, ConnectionStatus, PluginCapabilities, PluginCommand, PluginError, PluginId,
};
use crate::chat::ChatState;
use crate::config::PluginConfig;
use crate::gateway::{self, ChatAttachment, GatewayCommand, GatewayEvent};

/// OpenClaw Gateway plugin — wraps the existing WebSocket gateway
/// connection logic from `gateway.rs`.
pub struct OpenClawPlugin {
    id: PluginId,
    plugin_name: String,
    url: String,
    token: Option<String>,
    status: Arc<Mutex<ConnectionStatus>>,
    /// Sender for gateway commands (chat.send, sessions.list).
    gateway_cmd_tx: Option<mpsc::UnboundedSender<GatewayCommand>>,
    /// Sender for plugin commands from the chat UI.
    plugin_cmd_tx: Option<mpsc::UnboundedSender<PluginCommand>>,
    /// Gateway event sender (forwarded to tray/monitor).
    gateway_event_tx: Option<mpsc::UnboundedSender<GatewayEvent>>,
    /// Shared chat state.
    chat_state: Arc<Mutex<ChatState>>,
    connected: bool,
}

impl OpenClawPlugin {
    pub fn new(config: &PluginConfig, chat_state: Arc<Mutex<ChatState>>) -> Self {
        let id_str = format!("openclaw-{}", slug(&config.name));
        Self {
            id: PluginId(id_str),
            plugin_name: config.name.clone(),
            url: config.url.clone().unwrap_or_default(),
            token: config.token.clone(),
            status: Arc::new(Mutex::new(ConnectionStatus::Disconnected)),
            gateway_cmd_tx: None,
            plugin_cmd_tx: None,
            gateway_event_tx: None,
            chat_state,
            connected: false,
        }
    }

    /// Set the gateway event sender so the plugin can forward events
    /// to the existing tray/monitor pipeline.
    pub fn set_gateway_event_tx(&mut self, tx: mpsc::UnboundedSender<GatewayEvent>) {
        self.gateway_event_tx = Some(tx);
    }

    /// Spawn the gateway connection and return whether it was started.
    fn spawn_gateway(&mut self) -> Result<(), PluginError> {
        let event_tx = self
            .gateway_event_tx
            .clone()
            .ok_or_else(|| PluginError("gateway event tx not set".to_string()))?;

        let (gw_cmd_tx, gw_cmd_rx) = mpsc::unbounded_channel();

        let status = Arc::clone(&self.status);
        let event_tx_status = event_tx.clone();
        let _conn_name = self.plugin_name.clone();

        // Wrap the event sender to track connection status
        let (proxy_tx, mut proxy_rx) = mpsc::unbounded_channel::<GatewayEvent>();

        // Spawn a task that intercepts gateway events to update plugin status
        let status_clone = Arc::clone(&status);
        tokio::spawn(async move {
            while let Some(event) = proxy_rx.recv().await {
                match &event {
                    GatewayEvent::Connected { .. } => {
                        if let Ok(mut s) = status_clone.lock() {
                            *s = ConnectionStatus::Connected;
                        }
                    }
                    GatewayEvent::Disconnected { .. } => {
                        if let Ok(mut s) = status_clone.lock() {
                            *s = ConnectionStatus::Reconnecting;
                        }
                    }
                    GatewayEvent::Error { .. } => {
                        if let Ok(mut s) = status_clone.lock() {
                            *s = ConnectionStatus::Error("gateway error".to_string());
                        }
                    }
                    _ => {}
                }
                // Forward to the real event channel
                let _ = event_tx_status.send(event);
            }
        });

        // Spawn plugin command translator
        let (plugin_cmd_tx, mut plugin_cmd_rx) = mpsc::unbounded_channel::<PluginCommand>();
        let gw_cmd_tx_clone = gw_cmd_tx.clone();
        tokio::spawn(async move {
            while let Some(cmd) = plugin_cmd_rx.recv().await {
                let gw_cmd = match cmd {
                    PluginCommand::SendChat {
                        message,
                        session_key,
                        attachments,
                    } => GatewayCommand::SendChat {
                        message,
                        session_key,
                        attachments,
                    },
                    PluginCommand::ListSessions => GatewayCommand::ListSessions,
                };
                let _ = gw_cmd_tx_clone.send(gw_cmd);
            }
        });

        // Use existing gateway::spawn_connection
        let url = self.url.clone();
        let token = self.token.clone();
        let name = self.plugin_name.clone();
        let chat_state = Arc::clone(&self.chat_state);

        let spawned = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(gateway::spawn_connection(
                name,
                Some(url),
                token,
                proxy_tx,
                chat_state,
                Some(gw_cmd_rx),
            ))
        });

        if !spawned {
            return Err(PluginError("gateway connection not started (empty URL?)".to_string()));
        }

        self.gateway_cmd_tx = Some(gw_cmd_tx);
        self.plugin_cmd_tx = Some(plugin_cmd_tx);
        self.connected = true;

        if let Ok(mut s) = self.status.lock() {
            *s = ConnectionStatus::Reconnecting; // connecting...
        }

        Ok(())
    }
}

impl AgentPlugin for OpenClawPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.plugin_name
    }

    fn plugin_type(&self) -> &str {
        "openclaw"
    }

    fn icon(&self) -> &str {
        "🦞"
    }

    fn capabilities(&self) -> PluginCapabilities {
        PluginCapabilities {
            chat: true,
            dashboard: true,
            workflows: false,
            logs: true,
        }
    }

    fn status(&self) -> ConnectionStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(ConnectionStatus::Disconnected)
    }

    fn connect(&mut self) -> Result<(), PluginError> {
        if self.connected {
            return Ok(());
        }
        self.spawn_gateway()
    }

    fn disconnect(&mut self) -> Result<(), PluginError> {
        // Drop the command sender — the gateway task will see the
        // channel close and wind down on its own.
        self.gateway_cmd_tx = None;
        self.plugin_cmd_tx = None;
        self.connected = false;
        if let Ok(mut s) = self.status.lock() {
            *s = ConnectionStatus::Disconnected;
        }
        Ok(())
    }

    fn send_message(
        &self,
        message: &str,
        session_key: Option<String>,
        attachments: Option<Vec<ChatAttachment>>,
    ) -> Result<(), PluginError> {
        let tx = self
            .gateway_cmd_tx
            .as_ref()
            .ok_or_else(|| PluginError("not connected".to_string()))?;
        tx.send(GatewayCommand::SendChat {
            message: message.to_string(),
            session_key,
            attachments,
        })
        .map_err(|e| PluginError(format!("send failed: {e}")))?;
        Ok(())
    }

    fn list_sessions(&self) -> Result<(), PluginError> {
        let tx = self
            .gateway_cmd_tx
            .as_ref()
            .ok_or_else(|| PluginError("not connected".to_string()))?;
        tx.send(GatewayCommand::ListSessions)
            .map_err(|e| PluginError(format!("list_sessions failed: {e}")))?;
        Ok(())
    }

    fn command_sender(&self) -> Option<mpsc::UnboundedSender<PluginCommand>> {
        self.plugin_cmd_tx.clone()
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
