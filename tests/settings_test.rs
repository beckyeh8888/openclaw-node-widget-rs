//! BDD-style tests for the Settings page IPC handlers and config CRUD.

use openclaw_node_widget_rs::chat::ChatState;
use openclaw_node_widget_rs::config::{Config, GeneralSettings, PluginConfig};
use openclaw_node_widget_rs::plugin::PluginCommand;
use std::sync::{Arc, Mutex};

// ── Feature: Plugin CRUD in Config ──────────────────────────────────

// Scenario: Add a new plugin via upsert_plugin
//   Given an empty Config
//   When upsert_plugin is called with a new PluginConfig
//   Then the plugin should appear in the plugins list
#[test]
fn scenario_add_new_plugin() {
    let mut config = Config::default();
    assert!(config.plugins.is_empty());

    config.upsert_plugin(PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Local Llama".to_string(),
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

    assert_eq!(config.plugins.len(), 1);
    assert_eq!(config.plugins[0].name, "Local Llama");
    assert_eq!(config.plugins[0].plugin_type, "ollama");
}

// Scenario: Update existing plugin via upsert_plugin
//   Given a Config with one plugin named "GPT"
//   When upsert_plugin is called with the same name but different model
//   Then the plugin should be updated (not duplicated)
#[test]
fn scenario_update_existing_plugin() {
    let mut config = Config::default();
    config.upsert_plugin(PluginConfig {
        plugin_type: "openai-compatible".to_string(),
        name: "GPT".to_string(),
        url: Some("https://api.openai.com/v1".to_string()),
        token: None,
        model: Some("gpt-4o".to_string()),
        api_key: Some("sk-old".to_string()),
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });

    // Update with new model
    config.upsert_plugin(PluginConfig {
        plugin_type: "openai-compatible".to_string(),
        name: "GPT".to_string(),
        url: Some("https://api.openai.com/v1".to_string()),
        token: None,
        model: Some("gpt-4o-mini".to_string()),
        api_key: Some("sk-new".to_string()),
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });

    assert_eq!(config.plugins.len(), 1, "should not duplicate");
    assert_eq!(config.plugins[0].model, Some("gpt-4o-mini".to_string()));
    assert_eq!(config.plugins[0].api_key, Some("sk-new".to_string()));
}

// Scenario: Remove a plugin by name
//   Given a Config with two plugins
//   When remove_plugin is called for one
//   Then only the other plugin should remain
#[test]
fn scenario_remove_plugin() {
    let mut config = Config::default();
    config.upsert_plugin(PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Alpha".to_string(),
        url: Some("http://localhost:11434".to_string()),
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });
    config.upsert_plugin(PluginConfig {
        plugin_type: "openai-compatible".to_string(),
        name: "Beta".to_string(),
        url: Some("https://api.openai.com/v1".to_string()),
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });
    assert_eq!(config.plugins.len(), 2);

    let removed = config.remove_plugin("Alpha");
    assert!(removed, "should return true");
    assert_eq!(config.plugins.len(), 1);
    assert_eq!(config.plugins[0].name, "Beta");
}

// Scenario: Remove a non-existent plugin returns false
//   Given a Config with one plugin
//   When remove_plugin is called with a different name
//   Then it should return false and leave the list unchanged
#[test]
fn scenario_remove_nonexistent_plugin() {
    let mut config = Config::default();
    config.upsert_plugin(PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Exists".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });

    let removed = config.remove_plugin("DoesNotExist");
    assert!(!removed);
    assert_eq!(config.plugins.len(), 1);
}

// Scenario: Add n8n plugin via upsert_plugin
//   Given a Config
//   When upsert_plugin is called with an n8n PluginConfig
//   Then the n8n plugin with webhook_url should be stored
#[test]
fn scenario_add_n8n_plugin() {
    let mut config = Config::default();
    config.upsert_plugin(PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "My Workflow".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: Some("https://n8n.example.com/webhook/abc".to_string()),
        poll_url: Some("https://n8n.example.com/webhook/abc/poll".to_string()),
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });

    assert_eq!(config.plugins.len(), 1);
    assert_eq!(config.plugins[0].plugin_type, "n8n");
    assert_eq!(
        config.plugins[0].webhook_url,
        Some("https://n8n.example.com/webhook/abc".to_string())
    );
    assert_eq!(
        config.plugins[0].poll_url,
        Some("https://n8n.example.com/webhook/abc/poll".to_string())
    );
}

// ── Feature: General Settings ───────────────────────────────────────

// Scenario: Apply general settings updates language and auto_start
//   Given a Config with default settings
//   When apply_general_settings is called with language="zh-tw" and auto_start=true
//   Then widget.language and startup.auto_start should be updated
#[test]
fn scenario_apply_general_settings() {
    let mut config = Config::default();
    assert_eq!(config.widget.language, "auto");
    assert!(!config.startup.auto_start);

    config.apply_general_settings(&GeneralSettings {
        language: "zh-tw".to_string(),
        auto_start: true,
        theme: "dark".to_string(),
        always_on_top: false,
    });

    assert_eq!(config.widget.language, "zh-tw");
    assert!(config.startup.auto_start);
}

// ── Feature: Settings IPC message handling ──────────────────────────

// Scenario: getSettings IPC sets settings_requested flag
//   Given a ChatState
//   When the getSettings IPC handler runs
//   Then settings_requested should be true
#[test]
fn scenario_get_settings_ipc() {
    let state = Arc::new(Mutex::new(ChatState::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<PluginCommand>();

    let body = r#"{"type":"getSettings"}"#;
    openclaw_node_widget_rs::chat::handle_ipc_message(body, &tx, &state);

    let s = state.lock().unwrap();
    assert!(s.settings_requested, "settings_requested should be set");
}

// Scenario: navigate to settings page sets current_page
//   Given a ChatState with current_page="chat"
//   When navigate IPC is received with page="settings"
//   Then current_page should be "settings"
#[test]
fn scenario_navigate_to_settings() {
    let state = Arc::new(Mutex::new(ChatState::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<PluginCommand>();

    let body = r#"{"type":"navigate","page":"settings"}"#;
    openclaw_node_widget_rs::chat::handle_ipc_message(body, &tx, &state);

    let s = state.lock().unwrap();
    assert_eq!(s.current_page, "settings");
}

// Scenario: Settings page init data includes settings_requested field
//   Given a new ChatState
//   Then settings_requested should default to false
#[test]
fn scenario_settings_requested_default() {
    let state = ChatState::new();
    assert!(!state.settings_requested);
}

// Scenario: Multiple plugin operations in sequence
//   Given an empty Config
//   When three plugins are added and one is removed
//   Then two plugins should remain in the correct order
#[test]
fn scenario_multiple_plugin_operations() {
    let mut config = Config::default();

    config.upsert_plugin(PluginConfig {
        plugin_type: "openclaw".to_string(),
        name: "First".to_string(),
        url: Some("ws://host1:18789".to_string()),
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });
    config.upsert_plugin(PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Second".to_string(),
        url: Some("http://localhost:11434".to_string()),
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });
    config.upsert_plugin(PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "Third".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: Some("https://n8n.example.com/webhook/xyz".to_string()),
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    });

    assert_eq!(config.plugins.len(), 3);

    config.remove_plugin("Second");
    assert_eq!(config.plugins.len(), 2);
    assert_eq!(config.plugins[0].name, "First");
    assert_eq!(config.plugins[1].name, "Third");
}
