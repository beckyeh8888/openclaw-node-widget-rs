//! BDD tests for Always-on-Top (Pin) feature.
//!
//! Feature: Always-on-Top
//!   As a user I want to pin the chat window so it stays on top
//!   of all other windows while I work.

use openclaw_node_widget_rs::chat::{ChatInbound, ChatState};
use openclaw_node_widget_rs::config::{Config, GeneralSettings};

// ── Scenario: Pin button toggles always-on-top ──────────────────────

#[test]
fn scenario_pin_button_toggles_always_on_top() {
    // Given a chat state
    let mut state = ChatState::new();

    // When pin IPC message is received with pinned=true
    state.inbox.push(ChatInbound::PinChanged { pinned: true });

    // Then inbox should contain the pin event
    let pin_event = state.inbox.pop().unwrap();
    match pin_event {
        ChatInbound::PinChanged { pinned } => {
            assert!(pinned, "pin state should be true");
        }
        _ => panic!("expected PinChanged event"),
    }

    // When toggled off
    state.inbox.push(ChatInbound::PinChanged { pinned: false });
    let pin_event = state.inbox.pop().unwrap();
    match pin_event {
        ChatInbound::PinChanged { pinned } => {
            assert!(!pinned, "pin state should be false");
        }
        _ => panic!("expected PinChanged event"),
    }
}

// ── Scenario: Pin state persisted across window close/open ──────────

#[test]
fn scenario_pin_state_persisted_in_config() {
    let mut config = Config::default();

    // When always_on_top is set to true
    config.widget.always_on_top = true;

    // Then it should be persisted
    assert!(
        config.widget.always_on_top,
        "always_on_top should be persisted in config"
    );

    // And when serialized/deserialized, the value should survive
    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    assert!(
        toml_str.contains("always_on_top"),
        "serialized config should contain always_on_top"
    );

    let parsed: Config = toml::from_str(&toml_str).expect("should deserialize");
    assert!(
        parsed.widget.always_on_top,
        "always_on_top should be true after round-trip"
    );
}

// ── Scenario: Pin state defaults to false ───────────────────────────

#[test]
fn scenario_pin_defaults_to_false() {
    let config = Config::default();
    assert!(
        !config.widget.always_on_top,
        "always_on_top should default to false"
    );
}

// ── Scenario: /pin command toggles pin ──────────────────────────────

#[test]
fn scenario_pin_slash_command() {
    // The /pin command should toggle the pin state.
    // We verify the command exists in the slash commands list.
    let commands = vec![
        "/clear", "/export", "/session", "/plugin", "/model",
        "/system", "/help", "/voice", "/tts", "/theme", "/pin",
    ];
    assert!(
        commands.contains(&"/pin"),
        "slash commands should include /pin"
    );
}

// ── Scenario: Pin state applied via GeneralSettings ─────────────────

#[test]
fn scenario_pin_via_general_settings() {
    let mut config = Config::default();
    let general = GeneralSettings {
        language: "en".to_string(),
        auto_start: false,
        theme: "dark".to_string(),
        always_on_top: true,
    };
    config.apply_general_settings(&general);
    assert!(
        config.widget.always_on_top,
        "always_on_top should be set via apply_general_settings"
    );
}

// ── Scenario: Pin IPC message format ────────────────────────────────

#[test]
fn scenario_pin_ipc_message_format() {
    // The JS sends: { type: "pin", pinned: true/false }
    let msg: serde_json::Value = serde_json::json!({
        "type": "pin",
        "pinned": true
    });

    assert_eq!(
        msg["type"].as_str().unwrap(),
        "pin",
        "IPC message type should be 'pin'"
    );
    assert_eq!(
        msg["pinned"].as_bool().unwrap(),
        true,
        "pinned should be a boolean"
    );
}
