use std::collections::HashMap;

use tokio::sync::mpsc;

use super::{
    AgentPlugin, ConnectionStatus, HealthStatus, PluginCommand, PluginError,
};
use crate::gateway::ChatAttachment;

/// Central registry that owns all loaded plugins.
pub struct PluginRegistry {
    plugins: HashMap<String, Box<dyn AgentPlugin>>,
    /// Insertion order for deterministic iteration.
    order: Vec<String>,
    active_plugin: Option<String>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            order: Vec::new(),
            active_plugin: None,
        }
    }

    /// Register a plugin.  If no active plugin is set, the first one
    /// registered becomes active.
    pub fn register(&mut self, plugin: Box<dyn AgentPlugin>) {
        let key = plugin.id().0.clone();
        if self.active_plugin.is_none() {
            self.active_plugin = Some(key.clone());
        }
        if !self.plugins.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.plugins.insert(key, plugin);
    }

    pub fn get(&self, id: &str) -> Option<&dyn AgentPlugin> {
        self.plugins.get(id).map(|b| b.as_ref())
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Box<dyn AgentPlugin>> {
        self.plugins.get_mut(id)
    }

    pub fn active(&self) -> Option<&dyn AgentPlugin> {
        self.active_plugin
            .as_ref()
            .and_then(|id| self.plugins.get(id))
            .map(|b| b.as_ref())
    }

    pub fn active_mut(&mut self) -> Option<&mut Box<dyn AgentPlugin>> {
        let id = self.active_plugin.clone()?;
        self.plugins.get_mut(&id)
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active_plugin.as_deref()
    }

    pub fn set_active(&mut self, id: &str) -> Result<(), PluginError> {
        if !self.plugins.contains_key(id) {
            return Err(PluginError(format!("plugin not found: {id}")));
        }
        self.active_plugin = Some(id.to_string());
        Ok(())
    }

    /// Iterate all plugins in registration order.
    pub fn all(&self) -> Vec<&dyn AgentPlugin> {
        self.order
            .iter()
            .filter_map(|k| self.plugins.get(k))
            .map(|b| b.as_ref())
            .collect()
    }

    /// Plugin count.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Names in registration order.
    pub fn names(&self) -> Vec<String> {
        self.order
            .iter()
            .filter_map(|k| self.plugins.get(k))
            .map(|p| p.name().to_string())
            .collect()
    }

    /// IDs in registration order.
    pub fn ids(&self) -> Vec<String> {
        self.order.clone()
    }

    /// Connect all registered plugins.
    pub fn connect_all(&mut self) {
        for key in &self.order {
            if let Some(plugin) = self.plugins.get_mut(key) {
                if let Err(e) = plugin.connect() {
                    tracing::warn!(plugin = %key, error = %e, "plugin connect failed");
                }
            }
        }
    }

    /// Disconnect all registered plugins.
    pub fn disconnect_all(&mut self) {
        for key in &self.order {
            if let Some(plugin) = self.plugins.get_mut(key) {
                let _ = plugin.disconnect();
            }
        }
    }

    /// Get the command sender for the active plugin (for chat UI).
    pub fn active_command_sender(&self) -> Option<mpsc::UnboundedSender<PluginCommand>> {
        self.active().and_then(|p| p.command_sender())
    }

    /// Send a message through the active plugin.
    pub fn send_message(
        &self,
        message: &str,
        session_key: Option<String>,
        attachments: Option<Vec<ChatAttachment>>,
    ) -> Result<(), PluginError> {
        let plugin = self.active().ok_or_else(|| PluginError("no active plugin".to_string()))?;
        plugin.send_message(message, session_key, attachments)
    }

    /// Request session list from the active plugin.
    pub fn list_sessions(&self) -> Result<(), PluginError> {
        let plugin = self.active().ok_or_else(|| PluginError("no active plugin".to_string()))?;
        plugin.list_sessions()
    }

    /// Run health checks on all plugins and return results.
    pub fn health_check_all(&self) -> Vec<(String, HealthStatus)> {
        self.order
            .iter()
            .filter_map(|k| {
                self.plugins.get(k).map(|p| {
                    (p.id().0.clone(), p.health_check())
                })
            })
            .collect()
    }

    /// Get status info for all plugins (for tray display).
    pub fn plugin_statuses(&self) -> Vec<(String, String, ConnectionStatus)> {
        self.order
            .iter()
            .filter_map(|k| {
                self.plugins.get(k).map(|p| {
                    (p.id().0.clone(), p.name().to_string(), p.status())
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{PluginCapabilities, PluginId};

    /// Minimal test plugin for registry tests.
    struct TestPlugin {
        id: PluginId,
        name: String,
        status: ConnectionStatus,
    }

    impl TestPlugin {
        fn new(id: &str, name: &str) -> Self {
            Self {
                id: PluginId(id.to_string()),
                name: name.to_string(),
                status: ConnectionStatus::Disconnected,
            }
        }
    }

    impl AgentPlugin for TestPlugin {
        fn id(&self) -> &PluginId { &self.id }
        fn name(&self) -> &str { &self.name }
        fn plugin_type(&self) -> &str { "test" }
        fn icon(&self) -> &str { "🧪" }
        fn capabilities(&self) -> PluginCapabilities { PluginCapabilities::default() }
        fn status(&self) -> ConnectionStatus { self.status.clone() }
        fn connect(&mut self) -> Result<(), PluginError> {
            self.status = ConnectionStatus::Connected;
            Ok(())
        }
        fn disconnect(&mut self) -> Result<(), PluginError> {
            self.status = ConnectionStatus::Disconnected;
            Ok(())
        }
        fn send_message(&self, _msg: &str, _sk: Option<String>, _att: Option<Vec<ChatAttachment>>) -> Result<(), PluginError> {
            Ok(())
        }
        fn list_sessions(&self) -> Result<(), PluginError> { Ok(()) }
        fn command_sender(&self) -> Option<mpsc::UnboundedSender<PluginCommand>> { None }
    }

    #[test]
    fn given_empty_registry_then_no_active_plugin() {
        let reg = PluginRegistry::new();
        assert!(reg.active().is_none());
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn given_one_plugin_registered_then_it_becomes_active() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("p1", "Plugin 1")));

        assert_eq!(reg.len(), 1);
        assert_eq!(reg.active_id(), Some("p1"));
        assert_eq!(reg.active().unwrap().name(), "Plugin 1");
    }

    #[test]
    fn given_two_plugins_then_first_is_active_by_default() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("a", "Alpha")));
        reg.register(Box::new(TestPlugin::new("b", "Beta")));

        assert_eq!(reg.active_id(), Some("a"));
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn given_two_plugins_when_set_active_then_active_changes() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("a", "Alpha")));
        reg.register(Box::new(TestPlugin::new("b", "Beta")));

        reg.set_active("b").unwrap();
        assert_eq!(reg.active_id(), Some("b"));
        assert_eq!(reg.active().unwrap().name(), "Beta");
    }

    #[test]
    fn given_registry_when_set_active_unknown_then_error() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("a", "Alpha")));

        let result = reg.set_active("unknown");
        assert!(result.is_err());
    }

    #[test]
    fn given_plugins_when_connect_all_then_all_connected() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("a", "Alpha")));
        reg.register(Box::new(TestPlugin::new("b", "Beta")));

        reg.connect_all();

        for p in reg.all() {
            assert_eq!(p.status(), ConnectionStatus::Connected);
        }
    }

    #[test]
    fn given_connected_plugins_when_disconnect_all_then_all_disconnected() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("a", "Alpha")));
        reg.connect_all();

        reg.disconnect_all();

        for p in reg.all() {
            assert_eq!(p.status(), ConnectionStatus::Disconnected);
        }
    }

    #[test]
    fn given_plugins_then_names_returns_in_order() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("b", "Beta")));
        reg.register(Box::new(TestPlugin::new("a", "Alpha")));

        let names = reg.names();
        assert_eq!(names, vec!["Beta", "Alpha"]);
    }

    #[test]
    fn given_plugins_then_all_returns_in_order() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("first", "First")));
        reg.register(Box::new(TestPlugin::new("second", "Second")));

        let all = reg.all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name(), "First");
        assert_eq!(all[1].name(), "Second");
    }

    #[test]
    fn given_plugins_then_statuses_returns_all() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("a", "Alpha")));
        reg.register(Box::new(TestPlugin::new("b", "Beta")));

        let statuses = reg.plugin_statuses();
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].0, "a");
        assert_eq!(statuses[0].2, ConnectionStatus::Disconnected);
    }
}
