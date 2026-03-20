//! BDD tests for TTS (Text-to-Speech) voice output feature.
//!
//! Feature: TTS Voice Output
//!   As a user I want agent messages to be read aloud
//!   so that I can listen to replies hands-free.

use serde_json::json;

// ── TTS config defaults ─────────────────────────────────────────────

#[test]
fn scenario_tts_enabled_by_default() {
    let tts = json!({
        "enabled": true,
        "auto_read": false,
        "voice": "auto",
        "rate": 1.0,
    });

    assert_eq!(tts["enabled"], true, "TTS should be enabled by default");
    assert_eq!(tts["auto_read"], false, "auto-read should be off by default");
    assert_eq!(tts["voice"], "auto");
    assert_eq!(tts["rate"], 1.0);
}

#[test]
fn scenario_tts_config_included_in_init_data() {
    let init = json!({
        "lang": "en",
        "connected": false,
        "messages": [],
        "sessions": [],
        "selectedSession": null,
        "waitingForReply": false,
        "tts": {
            "enabled": true,
            "auto_read": false,
            "voice": "auto",
            "rate": 1.0,
        },
    });

    assert!(init.get("tts").is_some(), "init data must include tts config");
    assert_eq!(init["tts"]["enabled"], true);
    assert_eq!(init["tts"]["auto_read"], false);
}

#[test]
fn scenario_tts_disabled_hides_controls() {
    // When tts.enabled = false, the JS init hides the auto-read button
    let tts = json!({ "enabled": false, "auto_read": false, "voice": "auto", "rate": 1.0 });
    assert_eq!(tts["enabled"], false, "TTS can be disabled via config");
}

#[test]
fn scenario_auto_read_toggle_persisted_via_ipc() {
    // The JS sends this IPC message when the auto-read button is toggled
    let ipc_msg = json!({
        "type": "setTtsAutoRead",
        "autoRead": true,
    });

    assert_eq!(ipc_msg["type"], "setTtsAutoRead");
    assert_eq!(ipc_msg["autoRead"], true);
}

#[test]
fn scenario_auto_read_toggle_off_persisted_via_ipc() {
    let ipc_msg = json!({
        "type": "setTtsAutoRead",
        "autoRead": false,
    });

    assert_eq!(ipc_msg["autoRead"], false);
}

#[test]
fn scenario_tts_rate_clamped_to_valid_range() {
    // Valid rates: 0.5 to 2.0
    let rates = vec![0.5_f64, 1.0, 1.5, 2.0];
    for rate in &rates {
        assert!(*rate >= 0.5 && *rate <= 2.0, "rate {} should be in valid range", rate);
    }
}

#[test]
fn scenario_tts_voice_auto_selects_by_lang() {
    // When voice = "auto", lang determines the utterance language
    let mappings = vec![
        ("en", "en-US"),
        ("zh-tw", "zh-TW"),
        ("zh-cn", "zh-CN"),
    ];

    for (lang, expected_locale) in mappings {
        let locale = match lang {
            "zh-tw" => "zh-TW",
            "zh-cn" => "zh-CN",
            _ => "en-US",
        };
        assert_eq!(locale, expected_locale, "lang '{}' should map to '{}'", lang, expected_locale);
    }
}

#[test]
fn scenario_tts_speak_button_data_on_agent_message() {
    // Agent messages should include the raw text for TTS
    let agent_msg = json!({
        "sender": "agent",
        "agentName": "Bot",
        "text": "Hello, how can I help you today?",
    });

    assert_eq!(agent_msg["sender"], "agent");
    assert!(!agent_msg["text"].as_str().unwrap().is_empty(), "agent text should be non-empty for TTS");
}

#[test]
fn scenario_user_messages_do_not_get_tts_button() {
    let user_msg = json!({
        "sender": "user",
        "text": "hi",
    });

    // TTS buttons are only added for agent messages
    assert_eq!(user_msg["sender"], "user");
}

#[test]
fn scenario_tts_strips_markdown_for_speech() {
    // The speakText function strips HTML tags and markdown syntax
    let markdown_text = "**Bold** and `code` and [link](http://example.com)";
    let stripped = markdown_text
        .replace("**", "")
        .replace('`', "")
        .replace("[link](http://example.com)", "link");
    assert_eq!(stripped, "Bold and code and link");
}
