//! BDD-style tests for the OpenAI-Compatible plugin.

use std::sync::{Arc, Mutex};

use openclaw_node_widget_rs::chat::ChatState;
use openclaw_node_widget_rs::config::PluginConfig;
use openclaw_node_widget_rs::plugin::openai_compat::{
    build_auth_header, parse_openai_sse, OpenAICompatPlugin,
};
use openclaw_node_widget_rs::plugin::{AgentPlugin, ConnectionStatus};

// ── Feature: OpenAI-Compatible Plugin ───────────────────────────────

// Scenario: Parse SSE chat completion stream
//   Given SSE data with 3 delta chunks and [DONE]
//   When the response is parsed
//   Then 3 StreamChunk events should be emitted
//   And 1 MessageReceived with full text
#[test]
fn scenario_parse_sse_chat_completion_stream() {
    let sse_data = r#"data: {"choices":[{"delta":{"content":"Hello"}}]}

data: {"choices":[{"delta":{"content":" world"}}]}

data: {"choices":[{"delta":{"content":"!"}}]}

data: [DONE]
"#;

    let (chunks, full_text) = parse_openai_sse(sse_data);

    assert_eq!(chunks.len(), 3, "should emit 3 StreamChunk events");
    assert_eq!(chunks, vec!["Hello", " world", "!"]);
    assert_eq!(
        full_text,
        Some("Hello world!".to_string()),
        "should produce 1 MessageReceived with full text"
    );
}

// Scenario: Authorization header
//   Given a plugin with api_key "sk-test123"
//   When a request is made
//   Then Authorization header should be "Bearer sk-test123"
#[test]
fn scenario_authorization_header() {
    let key = Some("sk-test123".to_string());
    let header = build_auth_header(&key);
    assert_eq!(
        header,
        Some("Bearer sk-test123".to_string()),
        "Authorization header should be Bearer sk-test123"
    );
}

// Scenario: No api_key (local model)
//   Given a plugin with no api_key
//   When a request is made
//   Then no Authorization header should be present
#[test]
fn scenario_no_api_key_local_model() {
    let key: Option<String> = None;
    let header = build_auth_header(&key);
    assert_eq!(header, None, "no Authorization header should be present");
}

// Scenario: Config parsing
//   Given config with type="openai-compatible", url="https://api.openai.com/v1", model="gpt-4o", api_key="sk-..."
//   When parsed into PluginConfig
//   Then OpenAICompatPlugin should be created correctly
#[test]
fn scenario_config_parsing() {
    let config = PluginConfig {
        plugin_type: "openai-compatible".to_string(),
        name: "OpenAI GPT-4o".to_string(),
        url: Some("https://api.openai.com/v1".to_string()),
        token: None,
        model: Some("gpt-4o".to_string()),
        api_key: Some("sk-testkey".to_string()),
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
    };

    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = OpenAICompatPlugin::new(&config, chat_state);

    assert_eq!(plugin.name(), "OpenAI GPT-4o");
    assert_eq!(plugin.plugin_type(), "openai-compatible");
    assert_eq!(plugin.icon(), "🤖");
    assert!(plugin.capabilities().chat);
    assert!(!plugin.capabilities().dashboard);
    assert_eq!(plugin.status(), ConnectionStatus::Disconnected);
}

// Scenario: Plugin with LM Studio URL
#[test]
fn scenario_lm_studio_config() {
    let config = PluginConfig {
        plugin_type: "openai-compatible".to_string(),
        name: "LM Studio".to_string(),
        url: Some("http://localhost:1234/v1".to_string()),
        token: None,
        model: Some("local-model".to_string()),
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
    };

    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = OpenAICompatPlugin::new(&config, chat_state);

    assert_eq!(plugin.name(), "LM Studio");
}

// Scenario: SSE with role delta (first chunk often has role, no content)
#[test]
fn scenario_sse_role_delta_no_content() {
    let data = r#"data: {"choices":[{"delta":{"role":"assistant"}}]}
data: {"choices":[{"delta":{"content":"Hi"}}]}
data: [DONE]
"#;
    let (chunks, full) = parse_openai_sse(data);
    assert_eq!(chunks, vec!["Hi"]);
    assert_eq!(full, Some("Hi".to_string()));
}

// Scenario: Conversation history maintained
#[test]
fn scenario_conversation_history_maintained() {
    let history = vec![
        openclaw_node_widget_rs::plugin::openai_compat::OpenAIMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        },
        openclaw_node_widget_rs::plugin::openai_compat::OpenAIMessage {
            role: "assistant".to_string(),
            content: "hi".to_string(),
        },
    ];

    let messages = OpenAICompatPlugin::build_messages(&history, "new message");

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[1].content, "hi");
    assert_eq!(messages[2].role, "user");
    assert_eq!(messages[2].content, "new message");
}

// Scenario: Empty URL fails connect
#[test]
fn scenario_empty_url_fails() {
    let config = PluginConfig {
        plugin_type: "openai-compatible".to_string(),
        name: "Bad".to_string(),
        url: Some(String::new()),
        token: None,
        model: Some("gpt-4o".to_string()),
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let mut plugin = OpenAICompatPlugin::new(&config, chat_state);
    let result = plugin.connect();
    assert!(result.is_err());
}

// Scenario: Multiple choices in single SSE chunk
#[test]
fn scenario_multiple_choices() {
    let data = r#"data: {"choices":[{"delta":{"content":"a"}},{"delta":{"content":"b"}}]}
data: [DONE]
"#;
    let (chunks, full) = parse_openai_sse(data);
    assert_eq!(chunks, vec!["a", "b"]);
    assert_eq!(full, Some("ab".to_string()));
}
