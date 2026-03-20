//! BDD-style tests for the Ollama plugin.

use std::sync::{Arc, Mutex};

use openclaw_node_widget_rs::chat::ChatState;
use openclaw_node_widget_rs::config::PluginConfig;
use openclaw_node_widget_rs::plugin::ollama::{
    parse_ollama_ndjson, OllamaPlugin,
};
use openclaw_node_widget_rs::plugin::{AgentPlugin, ConnectionStatus};

// ── Feature: Ollama Plugin ──────────────────────────────────────────

// Scenario: Parse Ollama chat response
//   Given an NDJSON stream with 3 chunks and done=true
//   When the response is parsed
//   Then 3 StreamChunk events should be emitted
//   And 1 MessageReceived event with full concatenated text
#[test]
fn scenario_parse_ollama_chat_response() {
    let ndjson = r#"{"message":{"role":"assistant","content":"Hello"},"done":false}
{"message":{"role":"assistant","content":" there"},"done":false}
{"message":{"role":"assistant","content":"!"},"done":false}
{"message":{"role":"assistant","content":""},"done":true}
"#;

    let (chunks, full_text, _usage) = parse_ollama_ndjson(ndjson);

    assert_eq!(chunks.len(), 3, "should emit 3 StreamChunk events");
    assert_eq!(chunks, vec!["Hello", " there", "!"]);
    assert_eq!(
        full_text,
        Some("Hello there!".to_string()),
        "should produce 1 MessageReceived with full concatenated text"
    );
}

// Scenario: Connect validates URL
//   Given an Ollama plugin with url "http://localhost:11434"
//   When connect() is called and /api/tags returns 200
//   Then status should be Connected
//
// NOTE: This test uses a real connect, so it depends on Ollama running.
// We test the config + status logic instead.
#[test]
fn scenario_connect_validates_url_config() {
    let config = PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Test".to_string(),
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
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = OllamaPlugin::new(&config, chat_state);

    // Before connect, status is Disconnected
    assert_eq!(plugin.status(), ConnectionStatus::Disconnected);
    assert_eq!(plugin.plugin_type(), "ollama");
}

// Scenario: Connect fails on unreachable URL
//   Given an Ollama plugin with url "http://localhost:99999"
//   When connect() is called
//   Then status should be Error
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scenario_connect_fails_on_unreachable_url() {
    let config = PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Bad".to_string(),
        url: Some("http://localhost:99999".to_string()),
        token: None,
        model: Some("llama3.3".to_string()),
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let mut plugin = OllamaPlugin::new(&config, chat_state);

    let result = plugin.connect();
    assert!(result.is_err(), "connect to unreachable URL should fail");
    match plugin.status() {
        ConnectionStatus::Error(_) => {} // expected
        other => panic!("expected Error status, got: {other:?}"),
    }
}

// Scenario: Connect fails on empty URL
#[test]
fn scenario_connect_fails_on_empty_url() {
    let config = PluginConfig {
        plugin_type: "ollama".to_string(),
        name: "Empty".to_string(),
        url: Some(String::new()),
        token: None,
        model: Some("llama3.3".to_string()),
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let mut plugin = OllamaPlugin::new(&config, chat_state);

    let result = plugin.connect();
    assert!(result.is_err());
}

// Scenario: Conversation history maintained
//   Given an Ollama plugin with 2 previous messages
//   When user sends a 3rd message
//   Then the API request should include all 3 messages
#[test]
fn scenario_conversation_history_maintained() {
    use openclaw_node_widget_rs::plugin::ollama::OllamaPlugin;

    let history = vec![
        openclaw_node_widget_rs::plugin::ollama::OllamaMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        },
        openclaw_node_widget_rs::plugin::ollama::OllamaMessage {
            role: "assistant".to_string(),
            content: "hi there".to_string(),
        },
    ];

    let messages = OllamaPlugin::build_messages(None, &history, "how are you?");

    assert_eq!(
        messages.len(),
        3,
        "API request should include all 3 messages"
    );
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "hi there");
    assert_eq!(messages[2].role, "user");
    assert_eq!(messages[2].content, "how are you?");
}

// Scenario: Config parsing
//   Given config with type="ollama", url="http://localhost:11434", model="llama3.3"
//   When parsed into PluginConfig
//   Then OllamaPlugin should be created with correct url and model
#[test]
fn scenario_config_parsing() {
    let config = PluginConfig {
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
    };

    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = OllamaPlugin::new(&config, chat_state);

    assert_eq!(plugin.name(), "Local Llama");
    assert_eq!(plugin.plugin_type(), "ollama");
    assert_eq!(plugin.icon(), "🦙");
    assert!(plugin.capabilities().chat);
    assert!(!plugin.capabilities().dashboard);
}

// Scenario: NDJSON with only done line
#[test]
fn scenario_ndjson_done_only() {
    let data = r#"{"done":true}
"#;
    let (chunks, full, _usage) = parse_ollama_ndjson(data);
    assert!(chunks.is_empty());
    assert_eq!(full, Some(String::new()));
}

// Scenario: Multiple rapid chunks
#[test]
fn scenario_ndjson_many_chunks() {
    let mut lines = String::new();
    for i in 0..10 {
        lines.push_str(&format!(
            r#"{{"message":{{"role":"assistant","content":"w{i}"}},"done":false}}"#
        ));
        lines.push('\n');
    }
    lines.push_str(r#"{"done":true}"#);
    lines.push('\n');

    let (chunks, full, _usage) = parse_ollama_ndjson(&lines);
    assert_eq!(chunks.len(), 10);
    assert!(full.is_some());
    assert_eq!(full.unwrap(), "w0w1w2w3w4w5w6w7w8w9");
}
