//! BDD-style tests for System Prompt per Plugin feature.

use openclaw_node_widget_rs::config::PluginConfig;
use openclaw_node_widget_rs::plugin::ollama::{OllamaMessage, OllamaPlugin};
use openclaw_node_widget_rs::plugin::openai_compat::{OpenAICompatPlugin, OpenAIMessage};

// ── Feature: System Prompt ──────────────────────────────────────────

// Scenario: System prompt from config applied to Ollama requests
//   Given a plugin config with system_prompt = "You are a coder"
//   When build_messages is called
//   Then the first message should be a system message with that prompt
#[test]
fn scenario_system_prompt_applied_to_ollama() {
    let history = vec![
        OllamaMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        },
    ];
    let messages = OllamaPlugin::build_messages(
        Some("You are a coder"),
        &history,
        "write a function",
    );
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, "You are a coder");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content, "hello");
    assert_eq!(messages[2].role, "user");
    assert_eq!(messages[2].content, "write a function");
}

// Scenario: System prompt from config applied to OpenAI requests
//   Given a plugin config with system_prompt = "You are a helpful assistant"
//   When build_messages is called
//   Then the first message should be a system message
#[test]
fn scenario_system_prompt_applied_to_openai() {
    let history = vec![
        OpenAIMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        },
    ];
    let messages = OpenAICompatPlugin::build_messages(
        Some("You are a helpful assistant"),
        &history,
        "how are you?",
    );
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, "You are a helpful assistant");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content, "hi");
    assert_eq!(messages[2].role, "user");
    assert_eq!(messages[2].content, "how are you?");
}

// Scenario: Empty system prompt means no system message
//   Given a plugin config with system_prompt = ""
//   When build_messages is called
//   Then no system message is prepended
#[test]
fn scenario_empty_system_prompt_no_message_ollama() {
    let messages = OllamaPlugin::build_messages(Some(""), &[], "hello");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
}

#[test]
fn scenario_empty_system_prompt_no_message_openai() {
    let messages = OpenAICompatPlugin::build_messages(Some(""), &[], "hello");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
}

// Scenario: None system prompt means no system message
//   Given a plugin config with no system_prompt
//   When build_messages is called
//   Then no system message is prepended
#[test]
fn scenario_none_system_prompt_no_message_ollama() {
    let messages = OllamaPlugin::build_messages(None, &[], "hello");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
}

#[test]
fn scenario_none_system_prompt_no_message_openai() {
    let messages = OpenAICompatPlugin::build_messages(None, &[], "hello");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
}

// Scenario: System prompt saved in config
//   Given a PluginConfig with system_prompt set
//   When the config is serialized and deserialized
//   Then the system_prompt is preserved
#[test]
fn scenario_system_prompt_roundtrip_in_config() {
    let config = PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Coder".to_string(),
        url: Some("http://localhost:11434".to_string()),
        token: None,
        model: Some("codellama".to_string()),
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: Some("You are an expert programmer.".to_string()),
    };
    let toml_str = toml::to_string(&config).unwrap();
    assert!(toml_str.contains("system_prompt"));
    let parsed: PluginConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(
        parsed.system_prompt,
        Some("You are an expert programmer.".to_string())
    );
}

// Scenario: System prompt absent from config defaults to None
//   Given a TOML config without system_prompt field
//   When parsed
//   Then system_prompt is None
#[test]
fn scenario_system_prompt_defaults_to_none() {
    let toml_str = r#"
type = "ollama"
name = "Test"
"#;
    let parsed: PluginConfig = toml::from_str(toml_str).unwrap();
    assert!(parsed.system_prompt.is_none());
}

// Scenario: System prompt with history preserves conversation context
//   Given a system prompt and existing conversation history
//   When build_messages is called
//   Then system prompt is first, then history, then new message
#[test]
fn scenario_system_prompt_ordering_with_history() {
    let history = vec![
        OllamaMessage {
            role: "user".to_string(),
            content: "what is rust?".to_string(),
        },
        OllamaMessage {
            role: "assistant".to_string(),
            content: "Rust is a systems programming language.".to_string(),
        },
    ];
    let messages = OllamaPlugin::build_messages(
        Some("Be concise."),
        &history,
        "tell me more",
    );
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, "Be concise.");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[2].role, "assistant");
    assert_eq!(messages[3].role, "user");
    assert_eq!(messages[3].content, "tell me more");
}
