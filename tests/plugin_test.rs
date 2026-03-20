//! BDD-style integration tests for the Plugin system.
//!
//! These tests verify plugin registration, lifecycle, config migration,
//! and the registry routing logic without starting real network connections.

use openclaw_node_widget_rs::plugin::{
    registry::PluginRegistry,
    AgentPlugin, ConnectionStatus, PluginCapabilities, PluginCommand, PluginError, PluginId,
};
use tokio::sync::mpsc;

// ── Test helpers ────────────────────────────────────────────────────

/// A mock plugin for testing the registry and lifecycle.
struct MockPlugin {
    id: PluginId,
    name: String,
    plugin_type: String,
    capabilities: PluginCapabilities,
    status: ConnectionStatus,
    sent_messages: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    cmd_tx: Option<mpsc::UnboundedSender<PluginCommand>>,
}

impl MockPlugin {
    fn new(id: &str, name: &str, plugin_type: &str) -> Self {
        Self {
            id: PluginId(id.to_string()),
            name: name.to_string(),
            plugin_type: plugin_type.to_string(),
            capabilities: PluginCapabilities::default(),
            status: ConnectionStatus::Disconnected,
            sent_messages: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            cmd_tx: None,
        }
    }

    fn with_capabilities(mut self, caps: PluginCapabilities) -> Self {
        self.capabilities = caps;
        self
    }

    fn sent_messages(&self) -> Vec<String> {
        self.sent_messages.lock().unwrap().clone()
    }
}

impl AgentPlugin for MockPlugin {
    fn id(&self) -> &PluginId {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn plugin_type(&self) -> &str {
        &self.plugin_type
    }
    fn icon(&self) -> &str {
        "🧪"
    }
    fn capabilities(&self) -> PluginCapabilities {
        self.capabilities.clone()
    }
    fn status(&self) -> ConnectionStatus {
        self.status.clone()
    }
    fn connect(&mut self) -> Result<(), PluginError> {
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
        _attachments: Option<Vec<openclaw_node_widget_rs::gateway::ChatAttachment>>,
    ) -> Result<(), PluginError> {
        self.sent_messages.lock().unwrap().push(message.to_string());
        Ok(())
    }
    fn list_sessions(&self) -> Result<(), PluginError> {
        Ok(())
    }
    fn command_sender(&self) -> Option<mpsc::UnboundedSender<PluginCommand>> {
        self.cmd_tx.clone()
    }
}

// ── Feature: Plugin System ──────────────────────────────────────────

// Scenario: Load plugins from config
//   Given config.toml contains 2 plugin entries (openclaw + ollama)
//   When the widget starts
//   Then PluginRegistry should contain 2 plugins
//   And each plugin should be in Disconnected state
#[test]
fn scenario_load_plugins_from_config() {
    let mut registry = PluginRegistry::new();

    let openclaw = MockPlugin::new("openclaw-home", "Home", "openclaw");
    let ollama = MockPlugin::new("ollama-local", "Local Llama", "ollama");

    registry.register(Box::new(openclaw));
    registry.register(Box::new(ollama));

    assert_eq!(registry.len(), 2, "registry should contain 2 plugins");

    for plugin in registry.all() {
        assert_eq!(
            plugin.status(),
            ConnectionStatus::Disconnected,
            "each plugin should be in Disconnected state"
        );
    }
}

// Scenario: Plugin connects successfully
//   Given an OpenClaw plugin is registered
//   When the plugin connects
//   Then the plugin status should be Connected
#[test]
fn scenario_plugin_connects_successfully() {
    let mut registry = PluginRegistry::new();
    registry.register(Box::new(MockPlugin::new("oc-1", "Arno", "openclaw")));

    registry.connect_all();

    let plugin = registry.get("oc-1").unwrap();
    assert_eq!(
        plugin.status(),
        ConnectionStatus::Connected,
        "plugin should be Connected after connect()"
    );
}

// Scenario: Send message via plugin
//   Given the OpenClaw plugin is connected
//   When user sends "Hello" to the OpenClaw plugin
//   Then the plugin's send() method should be called with "Hello"
#[test]
fn scenario_send_message_via_plugin() {
    let mut registry = PluginRegistry::new();
    let mock = MockPlugin::new("oc-1", "Arno", "openclaw");
    let sent = mock.sent_messages.clone();
    registry.register(Box::new(mock));
    registry.connect_all();

    registry.send_message("Hello", None, None).unwrap();

    let messages = sent.lock().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], "Hello");
}

// Scenario: Plugin disconnects and reconnects
//   Given the OpenClaw plugin is connected
//   When disconnect is called
//   Then the plugin status should be Disconnected
//   When connect is called again
//   Then the plugin status should be Connected
#[test]
fn scenario_plugin_disconnect_and_reconnect() {
    let mut registry = PluginRegistry::new();
    registry.register(Box::new(MockPlugin::new("oc-1", "Arno", "openclaw")));
    registry.connect_all();

    assert_eq!(
        registry.get("oc-1").unwrap().status(),
        ConnectionStatus::Connected
    );

    registry.disconnect_all();
    assert_eq!(
        registry.get("oc-1").unwrap().status(),
        ConnectionStatus::Disconnected
    );

    registry.connect_all();
    assert_eq!(
        registry.get("oc-1").unwrap().status(),
        ConnectionStatus::Connected
    );
}

// Scenario: Switch between plugins in chat
//   Given openclaw and ollama plugins are both connected
//   When user switches active plugin to ollama
//   Then the active plugin should be ollama
//   And new messages should be sent via the ollama plugin
#[test]
fn scenario_switch_between_plugins() {
    let mut registry = PluginRegistry::new();
    let oc = MockPlugin::new("oc-1", "Arno", "openclaw");
    let ollama = MockPlugin::new("ollama-1", "Local Llama", "ollama");
    let ollama_sent = ollama.sent_messages.clone();

    registry.register(Box::new(oc));
    registry.register(Box::new(ollama));
    registry.connect_all();

    // Default active is the first registered plugin
    assert_eq!(registry.active_id(), Some("oc-1"));

    // Switch to ollama
    registry.set_active("ollama-1").unwrap();
    assert_eq!(registry.active_id(), Some("ollama-1"));
    assert_eq!(registry.active().unwrap().name(), "Local Llama");

    // Send through active plugin
    registry.send_message("test", None, None).unwrap();
    let messages = ollama_sent.lock().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], "test");
}

// Scenario: Plugin with chat + dashboard capabilities
//   Given the OpenClaw plugin has capabilities {chat: true, dashboard: true}
//   Then the capabilities should reflect both
#[test]
fn scenario_plugin_capabilities_chat_and_dashboard() {
    let plugin = MockPlugin::new("oc-1", "Arno", "openclaw").with_capabilities(
        PluginCapabilities {
            chat: true,
            dashboard: true,
            workflows: false,
            logs: false,
        },
    );

    let caps = plugin.capabilities();
    assert!(caps.chat, "should have chat capability");
    assert!(caps.dashboard, "should have dashboard capability");
}

// Scenario: Plugin without dashboard capability
//   Given the Ollama plugin has capabilities {chat: true, dashboard: false}
//   Then the capabilities should reflect no dashboard
#[test]
fn scenario_plugin_without_dashboard() {
    let plugin = MockPlugin::new("ollama-1", "Local", "ollama").with_capabilities(
        PluginCapabilities {
            chat: true,
            dashboard: false,
            workflows: false,
            logs: false,
        },
    );

    let caps = plugin.capabilities();
    assert!(caps.chat);
    assert!(!caps.dashboard, "should NOT have dashboard capability");
}

// Scenario: First registered plugin is active by default
#[test]
fn scenario_first_plugin_is_active() {
    let mut registry = PluginRegistry::new();
    registry.register(Box::new(MockPlugin::new("a", "Alpha", "test")));
    registry.register(Box::new(MockPlugin::new("b", "Beta", "test")));

    assert_eq!(registry.active_id(), Some("a"));
}

// Scenario: Setting unknown plugin as active returns error
#[test]
fn scenario_set_active_unknown_plugin_error() {
    let mut registry = PluginRegistry::new();
    registry.register(Box::new(MockPlugin::new("a", "Alpha", "test")));

    let result = registry.set_active("nonexistent");
    assert!(result.is_err());
}

// Scenario: Send message with no active plugin returns error
#[test]
fn scenario_send_message_no_active_plugin() {
    let registry = PluginRegistry::new();
    let result = registry.send_message("hello", None, None);
    assert!(result.is_err());
}

// Scenario: Plugin statuses for tray display
#[test]
fn scenario_plugin_statuses_for_tray() {
    let mut registry = PluginRegistry::new();
    registry.register(Box::new(MockPlugin::new("oc-1", "Arno", "openclaw")));
    registry.register(Box::new(MockPlugin::new("ollama-1", "Local", "ollama")));
    registry.connect_all();

    let statuses = registry.plugin_statuses();
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0].0, "oc-1");
    assert_eq!(statuses[0].1, "Arno");
    assert_eq!(statuses[0].2, ConnectionStatus::Connected);
    assert_eq!(statuses[1].0, "ollama-1");
}

// Scenario: Plugin names in registration order
#[test]
fn scenario_plugin_names_ordered() {
    let mut registry = PluginRegistry::new();
    registry.register(Box::new(MockPlugin::new("z", "Zulu", "test")));
    registry.register(Box::new(MockPlugin::new("a", "Alpha", "test")));

    let names = registry.names();
    assert_eq!(names, vec!["Zulu", "Alpha"]);
}

// Scenario: Config migration from [[connections]] to [[plugins]]
#[test]
fn scenario_config_migration_connections_to_plugins() {
    use openclaw_node_widget_rs::config::{Config, ConnectionConfig};

    let mut config = Config::default();
    config.connections.push(ConnectionConfig {
        name: "Home".to_string(),
        gateway_url: "ws://192.168.1.100:18789".to_string(),
        gateway_token: Some("test-token".to_string()),
    });
    config.connections.push(ConnectionConfig {
        name: "Work".to_string(),
        gateway_url: "wss://work.example.com:18789".to_string(),
        gateway_token: Some("work-token".to_string()),
    });

    let plugins = config.effective_plugins();
    assert_eq!(plugins.len(), 2, "should migrate 2 connections to 2 plugins");
    assert_eq!(plugins[0].plugin_type, "openclaw");
    assert_eq!(plugins[0].name, "Home");
    assert_eq!(
        plugins[0].url,
        Some("ws://192.168.1.100:18789".to_string())
    );
    assert_eq!(plugins[0].token, Some("test-token".to_string()));
    assert_eq!(plugins[1].plugin_type, "openclaw");
    assert_eq!(plugins[1].name, "Work");
}

// Scenario: Config with [[plugins]] takes precedence over [[connections]]
#[test]
fn scenario_config_plugins_take_precedence() {
    use openclaw_node_widget_rs::config::{Config, ConnectionConfig, PluginConfig};

    let mut config = Config::default();
    config.connections.push(ConnectionConfig {
        name: "Old".to_string(),
        gateway_url: "ws://old:18789".to_string(),
        gateway_token: None,
    });
    config.plugins.push(PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Local".to_string(),
        url: Some("http://localhost:11434".to_string()),
        token: None,
        model: Some("llama3.3".to_string()),
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });

    let plugins = config.effective_plugins();
    assert_eq!(plugins.len(), 1, "[[plugins]] should take precedence");
    assert_eq!(plugins[0].plugin_type, "ollama");
    assert_eq!(plugins[0].name, "Local");
}

// Scenario: Config with old [gateway] section migrates through connections to plugins
#[test]
fn scenario_config_gateway_to_plugins() {
    use openclaw_node_widget_rs::config::Config;

    let mut config = Config::default();
    config.gateway.url = Some("ws://localhost:18789".to_string());
    config.gateway.token = Some("my-token".to_string());

    let plugins = config.effective_plugins();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].plugin_type, "openclaw");
    assert_eq!(plugins[0].name, "Default");
    assert_eq!(
        plugins[0].url,
        Some("ws://localhost:18789".to_string())
    );
    assert_eq!(plugins[0].token, Some("my-token".to_string()));
}
