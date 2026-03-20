//! BDD-style tests for Wave 7: Smart Installer Flow.
//!
//! Tests cover: DefaultsConfig, detect_nodejs, wizard step flow,
//! connection test logic, pairing status, and new i18n keys.

use openclaw_node_widget_rs::config::Config;
use openclaw_node_widget_rs::i18n;

// ── Feature: Pre-configured Installer Defaults ──────────────────────

// Scenario: DefaultsConfig fields are optional and default to None
//   Given a default Config
//   Then defaults should have all None fields
#[test]
fn scenario_defaults_config_all_none() {
    let config = Config::default();
    assert!(config.defaults.gateway_host.is_none());
    assert!(config.defaults.gateway_port.is_none());
    assert!(config.defaults.gateway_token.is_none());
}

// Scenario: DefaultsConfig round-trips through TOML
//   Given a Config with defaults set
//   When serialized to TOML and deserialized
//   Then defaults should survive the round-trip
#[test]
fn scenario_defaults_config_toml_roundtrip() {
    let mut config = Config::default();
    config.defaults.gateway_host = Some("100.104.6.121".to_string());
    config.defaults.gateway_port = Some("18789".to_string());
    config.defaults.gateway_token = Some("secret-token".to_string());

    let toml_str = toml::to_string_pretty(&config).expect("serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("deserialize");

    assert_eq!(
        loaded.defaults.gateway_host,
        Some("100.104.6.121".to_string())
    );
    assert_eq!(loaded.defaults.gateway_port, Some("18789".to_string()));
    assert_eq!(
        loaded.defaults.gateway_token,
        Some("secret-token".to_string())
    );
}

// Scenario: Config without [defaults] section loads successfully
//   Given a TOML string with no [defaults] section
//   When deserialized
//   Then defaults should be all None
#[test]
fn scenario_config_without_defaults_section() {
    let toml_str = r#"
[gateway]
[node]
command = "openclaw node run"
[widget]
[startup]
[appearance]
[log]
"#;
    let config: Config = toml::from_str(toml_str).expect("deserialize");
    assert!(config.defaults.gateway_host.is_none());
    assert!(config.defaults.gateway_port.is_none());
    assert!(config.defaults.gateway_token.is_none());
}

// ── Feature: Auto-Install OpenClaw Node ─────────────────────────────

// Scenario: Node.js available, OpenClaw not installed
//   Given npm is available
//   And openclaw is not installed
//   When wizard reaches DetectInstall step
//   Then show "Install OpenClaw Node" button
//   (UI test — verified by existence of i18n keys and install flow logic)
#[test]
fn scenario_install_openclaw_i18n_keys_exist() {
    i18n::init();
    // All new keys required for the install flow should resolve to non-empty strings
    let keys = [
        "install_openclaw",
        "npm_available",
        "npm_not_found",
        "open_nodejs",
        "redetect",
        "install_nodejs",
        "nodejs_required",
        "nodejs_install_win",
        "nodejs_install_mac",
        "nodejs_install_linux",
        "installing",
        "install_failed",
        "retry",
    ];
    for key in &keys {
        let val = i18n::t(key);
        assert!(!val.is_empty(), "i18n key '{key}' should not be empty");
    }
}

// ── Feature: Tailscale Optional Step ────────────────────────────────

// Scenario: Tailscale i18n keys exist for all states
//   Given the i18n system is initialized
//   Then all Tailscale step keys should resolve
#[test]
fn scenario_tailscale_step_i18n_keys() {
    i18n::init();
    let keys = [
        "tailscale_step_title",
        "tailscale_optional_desc",
        "tailscale_install_btn",
        "tailscale_skip",
        "tailscale_disconnected_msg",
        "tailscale_open_btn",
        "tailscale_connected_label",
        "tailscale_select_gateway",
    ];
    for key in &keys {
        let val = i18n::t(key);
        assert!(!val.is_empty(), "i18n key '{key}' should not be empty");
    }
}

// ── Feature: Gateway Connection Test ────────────────────────────────

// Scenario: Gateway connection test i18n keys exist
//   Given the i18n system
//   Then connection test keys should resolve
#[test]
fn scenario_gateway_connection_test_i18n_keys() {
    i18n::init();
    let keys = [
        "test_connection",
        "connection_success",
        "connection_failed",
        "connection_failed_hint",
    ];
    for key in &keys {
        let val = i18n::t(key);
        assert!(!val.is_empty(), "i18n key '{key}' should not be empty");
    }
}

// ── Feature: Pairing Flow ───────────────────────────────────────────

// Scenario: Pairing step i18n keys exist
//   Given the i18n system
//   Then pairing step keys should resolve
#[test]
fn scenario_pairing_step_i18n_keys() {
    i18n::init();
    let keys = [
        "pairing_title",
        "pairing_checking",
        "pairing_waiting",
        "pairing_approved",
        "pairing_timeout",
        "pairing_already_paired",
    ];
    for key in &keys {
        let val = i18n::t(key);
        assert!(!val.is_empty(), "i18n key '{key}' should not be empty");
    }
}

// ── Feature: Complete Step ──────────────────────────────────────────

// Scenario: Complete step message
#[test]
fn scenario_complete_step_i18n() {
    i18n::init();
    i18n::set_language("en");
    let val = i18n::t("complete_msg");
    let lower = val.to_lowercase();
    assert!(
        lower.contains("complete") || lower.contains("setup") || lower.contains("monitoring"),
        "complete_msg should mention completion, got: {val}"
    );
}

// ── Feature: detect_nodejs utility ──────────────────────────────────

// Scenario: detect_nodejs returns a boolean
//   Given the setup module
//   When detect_nodejs is called
//   Then it returns true or false without panicking
#[test]
fn scenario_detect_nodejs_does_not_panic() {
    // This may return true or false depending on the system, but must not panic
    let _result = openclaw_node_widget_rs::config::detect_nodejs();
}

// ── Feature: Wizard step ordering ───────────────────────────────────

// Scenario: All wizard steps are defined in the expected order
//   (Verified by the WizardStep enum — tested indirectly via i18n keys)
#[test]
fn scenario_wizard_steps_have_titles() {
    i18n::init();
    let step_keys = [
        "welcome",
        "detect_install",
        "tailscale_step_title",
        "gateway_config",
        "pairing_title",
        "autostart",
        "complete",
    ];
    for key in &step_keys {
        let val = i18n::t(key);
        assert!(
            !val.is_empty(),
            "step title key '{key}' should not be empty"
        );
    }
}
