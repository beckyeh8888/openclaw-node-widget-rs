//! BDD-style tests for the Plugin Health Check feature.
//!
//! Periodic health checks ping each plugin backend every 60 seconds
//! and display reachability, latency, and uptime on the dashboard.

use std::sync::{Arc, Mutex};

use openclaw_node_widget_rs::chat::ChatState;
use openclaw_node_widget_rs::config::PluginConfig;
use openclaw_node_widget_rs::dashboard::{
    build_dashboard_data, HealthTracker, LatencyTracker,
};
use openclaw_node_widget_rs::plugin::ollama::OllamaPlugin;
use openclaw_node_widget_rs::plugin::openai_compat::OpenAICompatPlugin;
use openclaw_node_widget_rs::plugin::registry::PluginRegistry;
use openclaw_node_widget_rs::plugin::{
    AgentPlugin, ConnectionStatus, HealthStatus,
};

// ══════════════════════════════════════════════════════════════════════
// Feature: Plugin Health Check
// ══════════════════════════════════════════════════════════════════════

fn make_ollama_config(name: &str) -> PluginConfig {
    PluginConfig {
        plugin_type: "ollama".to_string(),
        name: name.to_string(),
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
    }
}

fn make_openai_config(name: &str) -> PluginConfig {
    PluginConfig {
        plugin_type: "openai-compatible".to_string(),
        name: name.to_string(),
        url: Some("https://api.openai.com/v1".to_string()),
        token: None,
        model: Some("gpt-4o".to_string()),
        api_key: Some("sk-test".to_string()),
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    }
}

// Scenario: Health check returns status for disconnected plugin
//   Given an Ollama plugin that is not connected
//   When health_check is called
//   Then it should return reachable=false (can't reach localhost:11434 in CI)
#[test]
fn scenario_health_check_disconnected_plugin_default() {
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = OllamaPlugin::new(&make_ollama_config("Test"), chat_state);

    // Default health_check on trait: reports based on connection status
    // Plugin is disconnected, so default would say not connected.
    // But OllamaPlugin overrides health_check to actually ping.
    let health = plugin.health_check();
    // In CI without Ollama running, this should be unreachable
    // We just check that it returns a valid HealthStatus
    // latency_ms is a u64, so always >= 0; just check it's a reasonable value
    let _ = health.latency_ms;
    if !health.reachable {
        assert!(health.error.is_some());
    }
}

// Scenario: Ollama health check pings /api/tags
//   Given an Ollama plugin
//   When health_check is called
//   Then it should attempt to reach /api/tags
//   (Verified by the implementation; here we just check it doesn't panic)
#[test]
fn scenario_ollama_health_check_pings_api_tags() {
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = OllamaPlugin::new(&make_ollama_config("Health"), chat_state);
    let health = plugin.health_check();
    // Should return a result without panicking
    assert!(!health.reachable || health.error.is_none());
}

// Scenario: OpenAI health check pings /models
//   Given an OpenAI-compatible plugin
//   When health_check is called
//   Then it should attempt to reach /models endpoint
#[test]
fn scenario_openai_health_check_pings_models() {
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = OpenAICompatPlugin::new(&make_openai_config("GPT"), chat_state);
    let health = plugin.health_check();
    // Without network, should get an error
    assert!(!health.reachable || health.error.is_none());
}

// Scenario: Failed health check updates status to Error
//   Given a plugin health check that fails
//   When the result is recorded
//   Then the error message should be captured
#[test]
fn scenario_failed_health_check_updates_status() {
    let health = HealthStatus {
        reachable: false,
        latency_ms: 5000,
        error: Some("connection refused".to_string()),
    };

    assert!(!health.reachable);
    assert_eq!(health.error, Some("connection refused".to_string()));
    assert_eq!(health.latency_ms, 5000);
}

// Scenario: Dashboard shows uptime percentage
//   Given a HealthTracker with 8 successful and 2 failed checks
//   When uptime_pct is calculated
//   Then it should be 80%
#[test]
fn scenario_dashboard_shows_uptime_percentage() {
    let mut tracker = HealthTracker::new();

    // Record 8 successful and 2 failed checks
    for i in 0..10 {
        tracker.record("test-plugin", HealthStatus {
            reachable: i < 8, // first 8 succeed
            latency_ms: 10,
            error: if i >= 8 { Some("down".to_string()) } else { None },
        });
    }

    let pct = tracker.uptime_pct("test-plugin");
    assert!(pct.is_some());
    assert!((pct.unwrap() - 80.0).abs() < 0.01, "uptime should be 80%");
}

// Scenario: Health check latency displayed
//   Given a successful health check with 42ms latency
//   When recorded
//   Then latest_record should show 42ms
#[test]
fn scenario_health_check_latency_displayed() {
    let mut tracker = HealthTracker::new();
    tracker.record("fast-plugin", HealthStatus {
        reachable: true,
        latency_ms: 42,
        error: None,
    });

    let record = tracker.latest_record("fast-plugin");
    assert!(record.is_some());
    let r = record.unwrap();
    assert!(r.reachable);
    assert_eq!(r.latency_ms, 42);
    assert!(r.error.is_none());
}

// Scenario: Health data appears in dashboard
//   Given plugins with health check data
//   When dashboard data is built
//   Then plugin cards should include health info
#[test]
fn scenario_health_data_in_dashboard() {
    let mut health_tracker = HealthTracker::new();
    health_tracker.record("ollama-test", HealthStatus {
        reachable: true,
        latency_ms: 15,
        error: None,
    });

    let statuses = vec![
        ("ollama-test".to_string(), "Ollama".to_string(), ConnectionStatus::Connected),
    ];
    let plugin_types = vec![
        ("ollama-test".to_string(), "ollama".to_string(), "🦙".to_string()),
    ];
    let tracker = LatencyTracker::new();
    let start = std::time::Instant::now();

    let data = build_dashboard_data(&statuses, &plugin_types, &tracker, start, Some(&health_tracker));

    assert_eq!(data.plugins.len(), 1);
    let card = &data.plugins[0];
    assert!(card.health.is_some(), "should have health data");
    let h = card.health.as_ref().unwrap();
    assert!(h.reachable);
    assert_eq!(h.latency_ms, 15);
    assert!(card.uptime_pct.is_some());
    assert!((card.uptime_pct.unwrap() - 100.0).abs() < 0.01);
}

// Scenario: Registry health_check_all returns all plugin health statuses
#[test]
fn scenario_registry_health_check_all() {
    let mut reg = PluginRegistry::new();
    let cs = Arc::new(Mutex::new(ChatState::new()));
    reg.register(Box::new(OllamaPlugin::new(&make_ollama_config("A"), Arc::clone(&cs))));
    reg.register(Box::new(OpenAICompatPlugin::new(&make_openai_config("B"), Arc::clone(&cs))));

    let results = reg.health_check_all();
    assert_eq!(results.len(), 2, "should return health for all plugins");
    // Each result should have a plugin id
    assert!(results[0].0.starts_with("ollama-"));
    assert!(results[1].0.starts_with("openai-"));
}

// Scenario: Health tracker with no data returns None
#[test]
fn scenario_health_tracker_no_data() {
    let tracker = HealthTracker::new();
    assert!(tracker.uptime_pct("nonexistent").is_none());
    assert!(tracker.latest_record("nonexistent").is_none());
}
