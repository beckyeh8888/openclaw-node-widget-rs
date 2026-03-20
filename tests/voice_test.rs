//! BDD-style tests for voice input (Whisper STT).

use openclaw_node_widget_rs::config::VoiceConfig;
use openclaw_node_widget_rs::voice::{transcribe, validate_audio_data};

// ── Feature: Voice Input ───────────────────────────────────────────

// Scenario: Audio sent as base64 via IPC — validate base64 decoding
//   Given valid base64-encoded audio data
//   When validated
//   Then the byte size should be returned
#[test]
fn scenario_valid_base64_audio_validated() {
    use base64::Engine;
    let data = vec![0u8; 1024];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
    let result = validate_audio_data(&encoded);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1024);
}

// Scenario: Empty audio rejected
//   Given empty audio data
//   When validated
//   Then an error should be returned
#[test]
fn scenario_empty_audio_rejected() {
    let result = validate_audio_data("");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty"));
}

// Scenario: Invalid base64 rejected
//   Given corrupted base64 data
//   When validated
//   Then an error should be returned
#[test]
fn scenario_invalid_base64_rejected() {
    let result = validate_audio_data("!!!not-base64!!!");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("base64"));
}

// Scenario: Recording too long (>30s) rejected
//   Given audio data exceeding 6MB
//   When validated
//   Then an error about size should be returned
#[test]
fn scenario_audio_too_large_rejected() {
    use base64::Engine;
    let data = vec![0u8; 7 * 1024 * 1024]; // 7MB > 6MB limit
    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
    let result = validate_audio_data(&encoded);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("too large"));
}

// Scenario: Whisper CLI transcription — empty input returns error
//   Given empty audio base64
//   When transcription is attempted
//   Then an error should be returned
#[tokio::test]
async fn scenario_transcribe_empty_input() {
    let config = VoiceConfig::default();
    let result = transcribe("", &config).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty"));
}

// Scenario: Whisper CLI transcription — invalid base64 returns error
//   Given invalid base64 audio
//   When transcription is attempted
//   Then a base64 error should be returned
#[tokio::test]
async fn scenario_transcribe_invalid_base64() {
    let config = VoiceConfig::default();
    let result = transcribe("!!!bad!!!", &config).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("base64"));
}

// Scenario: Voice disabled when not configured
//   Given a VoiceConfig with enabled = false
//   Then the config should reflect disabled state
#[test]
fn scenario_voice_disabled_by_default() {
    let config = VoiceConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.provider, "local");
    assert_eq!(config.language, "auto");
    assert!(config.openai_api_key.is_none());
}

// Scenario: Voice config with custom settings
//   Given a VoiceConfig with enabled=true and language="zh"
//   Then the config should reflect custom settings
#[test]
fn scenario_voice_custom_config() {
    let config = VoiceConfig {
        enabled: true,
        provider: "openai".to_string(),
        openai_api_key: Some("sk-test".to_string()),
        language: "zh".to_string(),
    };
    assert!(config.enabled);
    assert_eq!(config.provider, "openai");
    assert_eq!(config.language, "zh");
    assert_eq!(config.openai_api_key.as_deref(), Some("sk-test"));
}

// Scenario: VoiceConfig serialization roundtrip
#[test]
fn scenario_voice_config_serialization() {
    let config = VoiceConfig {
        enabled: true,
        provider: "local".to_string(),
        openai_api_key: None,
        language: "en".to_string(),
    };
    let toml_str = toml::to_string(&config).unwrap();
    let parsed: VoiceConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.enabled, true);
    assert_eq!(parsed.provider, "local");
    assert_eq!(parsed.language, "en");
}

// Scenario: Multiple small audio validations succeed
#[test]
fn scenario_multiple_valid_audio_sizes() {
    use base64::Engine;
    for size in [100, 1000, 10000, 100000] {
        let data = vec![42u8; size];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        let result = validate_audio_data(&encoded);
        assert!(result.is_ok(), "should accept {size} bytes");
        assert_eq!(result.unwrap(), size);
    }
}
