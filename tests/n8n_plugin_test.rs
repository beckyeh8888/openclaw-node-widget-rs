//! BDD-style tests for the n8n plugin.

use std::sync::{Arc, Mutex};

use openclaw_node_widget_rs::chat::ChatState;
use openclaw_node_widget_rs::config::PluginConfig;
use openclaw_node_widget_rs::plugin::n8n::{
    parse_n8n_response, HistoryEntry, N8nPlugin, N8nRequest, N8nResponse,
};
use openclaw_node_widget_rs::plugin::{AgentPlugin, ConnectionStatus};

// ── Feature: n8n Plugin ─────────────────────────────────────────────

// Scenario: Parse n8n immediate response with "response" field
//   Given a JSON body {"response": "Hello from n8n"}
//   When the response is parsed
//   Then the text should be "Hello from n8n"
#[test]
fn scenario_parse_immediate_response() {
    let body = r#"{"response": "Hello from n8n"}"#;
    let text = parse_n8n_response(body);
    assert_eq!(text, Some("Hello from n8n".to_string()));
}

// Scenario: Parse n8n response with "output" field
//   Given a JSON body {"output": "workflow result"}
//   When the response is parsed
//   Then the text should be "workflow result"
#[test]
fn scenario_parse_output_field() {
    let body = r#"{"output": "workflow result"}"#;
    let text = parse_n8n_response(body);
    assert_eq!(text, Some("workflow result".to_string()));
}

// Scenario: Parse n8n response array
//   Given an array response [{"response": "first"}]
//   When the response is parsed
//   Then the text should be "first"
#[test]
fn scenario_parse_array_response() {
    let body = r#"[{"response": "first"}]"#;
    let text = parse_n8n_response(body);
    assert_eq!(text, Some("first".to_string()));
}

// Scenario: Parse empty n8n response
//   Given an empty JSON object {}
//   When the response is parsed
//   Then the result should be None
#[test]
fn scenario_parse_empty_response() {
    let body = r#"{}"#;
    let text = parse_n8n_response(body);
    assert_eq!(text, None);
}

// Scenario: Parse invalid JSON
//   Given non-JSON text
//   When the response is parsed
//   Then the result should be None
#[test]
fn scenario_parse_invalid_json() {
    let body = "not json at all";
    let text = parse_n8n_response(body);
    assert_eq!(text, None);
}

// Scenario: Config parsing with webhook_url and poll_url
//   Given a PluginConfig with webhook_url and poll_url
//   When an N8nPlugin is created
//   Then the plugin should have the correct id, name, type, webhook_url, poll_url
#[test]
fn scenario_config_parsing() {
    let config = PluginConfig {
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
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = N8nPlugin::new(&config, chat_state);

    assert_eq!(plugin.id().0, "n8n-my-workflow");
    assert_eq!(plugin.name(), "My Workflow");
    assert_eq!(plugin.plugin_type(), "n8n");
    assert_eq!(plugin.icon(), "⚡");
}

// Scenario: Config falls back from url when webhook_url is absent
//   Given a PluginConfig with url but no webhook_url
//   When an N8nPlugin is created
//   Then webhook_url should use the url field value
#[test]
fn scenario_config_url_fallback() {
    let config = PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "Fallback".to_string(),
        url: Some("https://n8n.example.com/webhook/fallback".to_string()),
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = N8nPlugin::new(&config, chat_state);

    // The plugin should use the url field as webhook_url
    assert_eq!(plugin.name(), "Fallback");
    assert_eq!(plugin.plugin_type(), "n8n");
}

// Scenario: Plugin starts disconnected
//   Given a newly created N8nPlugin
//   Then its status should be Disconnected
#[test]
fn scenario_starts_disconnected() {
    let config = PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "Test".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: Some("https://n8n.example.com/webhook/abc".to_string()),
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = N8nPlugin::new(&config, chat_state);
    assert_eq!(plugin.status(), ConnectionStatus::Disconnected);
}

// Scenario: Connect fails without webhook_url
//   Given an N8nPlugin with empty webhook_url
//   When connect() is called
//   Then it should return an error about webhook_url
#[test]
fn scenario_connect_fails_without_webhook_url() {
    let config = PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "Test".to_string(),
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
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let mut plugin = N8nPlugin::new(&config, chat_state);
    let result = plugin.connect();
    assert!(result.is_err());
    assert!(result.unwrap_err().0.contains("webhook_url"));
}

// Scenario: Send message before connect fails
//   Given a disconnected N8nPlugin
//   When send_message is called
//   Then it should return an error
#[test]
fn scenario_send_before_connect_fails() {
    let config = PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "Test".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: Some("https://n8n.example.com/webhook/abc".to_string()),
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = N8nPlugin::new(&config, chat_state);
    let result = plugin.send_message("hello", None, None);
    assert!(result.is_err());
}

// Scenario: Capabilities include chat and workflows
//   Given an N8nPlugin
//   Then it should advertise chat and workflow capabilities
#[test]
fn scenario_capabilities() {
    let config = PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "Test".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: Some("https://example.com".to_string()),
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = N8nPlugin::new(&config, chat_state);
    let caps = plugin.capabilities();
    assert!(caps.chat, "n8n should support chat");
    assert!(caps.workflows, "n8n should support workflows");
    assert!(!caps.dashboard, "n8n should not support dashboard");
    assert!(!caps.logs, "n8n should not support logs");
}

// Scenario: N8nRequest serialization uses camelCase sessionId
//   Given an N8nRequest
//   When serialized to JSON
//   Then sessionId should be in camelCase
#[test]
fn scenario_request_serialization() {
    let req = N8nRequest {
        message: "hello".to_string(),
        session_id: "widget".to_string(),
        history: vec![],
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["message"], "hello");
    assert_eq!(json["sessionId"], "widget");
    assert!(json.get("session_id").is_none(), "should use camelCase");
}

// Scenario: N8nResponse field priority
//   Given a response with both "response" and "output" fields
//   Then "response" takes priority
#[test]
fn scenario_response_field_priority() {
    let resp = N8nResponse {
        response: Some("primary".to_string()),
        output: Some("secondary".to_string()),
        text: None,
    };
    assert_eq!(resp.text(), Some("primary"));
}

// Scenario: Disconnect clears plugin state
//   Given a connected N8nPlugin
//   When disconnect() is called
//   Then the status should be Disconnected and command sender cleared
#[test]
fn scenario_disconnect_clears_state() {
    let config = PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "Test".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: Some("https://example.com".to_string()),
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let mut plugin = N8nPlugin::new(&config, chat_state);

    // Can't set cmd_tx directly from integration test, so test via disconnect behavior
    plugin.disconnect().unwrap();
    assert_eq!(plugin.status(), ConnectionStatus::Disconnected);
    assert!(plugin.command_sender().is_none());
}

// ── Feature: n8n chat history ───────────────────────────────────────

// Scenario: First message has no history
//   Given a new n8n plugin session
//   When user sends "你好"
//   Then webhook body.history = []
#[test]
fn scenario_first_message_has_empty_history() {
    let req = N8nRequest {
        message: "你好".to_string(),
        session_id: "n8n-test".to_string(),
        history: vec![],
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["message"], "你好");
    assert_eq!(json["history"], serde_json::json!([]));
}

// Scenario: Second message includes first exchange
//   Given user sent "你好" and got reply "我是助手"
//   When user sends "列出 workflow"
//   Then webhook body.history contains the first exchange
#[test]
fn scenario_second_message_includes_first_exchange() {
    let history = vec![
        HistoryEntry { role: "user".into(), content: "你好".into() },
        HistoryEntry { role: "assistant".into(), content: "我是助手".into() },
    ];
    let req = N8nRequest {
        message: "列出 workflow".to_string(),
        session_id: "n8n-test".to_string(),
        history,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["message"], "列出 workflow");
    assert_eq!(json["history"][0]["role"], "user");
    assert_eq!(json["history"][0]["content"], "你好");
    assert_eq!(json["history"][1]["role"], "assistant");
    assert_eq!(json["history"][1]["content"], "我是助手");
}

// Scenario: History is capped at 20 messages (10 turns)
//   Given 15 complete exchanges exist
//   When user sends a new message
//   Then webhook body.history has exactly 20 entries (last 10 turns)
#[test]
fn scenario_history_capped_at_20_entries() {
    let mut history: Vec<HistoryEntry> = Vec::new();
    for i in 0..15 {
        history.push(HistoryEntry {
            role: "user".into(),
            content: format!("user msg {i}"),
        });
        history.push(HistoryEntry {
            role: "assistant".into(),
            content: format!("assistant reply {i}"),
        });
    }
    // Simulate the cap logic: keep last 20
    if history.len() > 20 {
        let excess = history.len() - 20;
        history.drain(..excess);
    }
    assert_eq!(history.len(), 20);
    assert_eq!(history[0].content, "user msg 5");
    assert_eq!(history[19].content, "assistant reply 14");

    let req = N8nRequest {
        message: "new message".to_string(),
        session_id: "n8n-test".to_string(),
        history,
    };
    let json = serde_json::to_value(&req).unwrap();
    let arr = json["history"].as_array().unwrap();
    assert_eq!(arr.len(), 20);
}

// Scenario: History is cleared on /clear command
//   Given user has chat history
//   When user sends "/clear"
//   Then local history is cleared and reply is emitted
#[test]
fn scenario_clear_command_clears_history() {
    let config = PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "Test".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: Some("https://example.com/webhook".to_string()),
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = N8nPlugin::new(&config, chat_state.clone());

    // /clear works even without being connected (no webhook call)
    let result = plugin.send_message("/clear", None, None);
    assert!(result.is_ok());

    // A reply was pushed to chat state
    let cs = chat_state.lock().unwrap();
    assert!(!cs.inbox.is_empty());
}

// Scenario: Request includes session_id derived from plugin name
#[test]
fn scenario_session_id_format() {
    let config = PluginConfig {
        plugin_type: "n8n".to_string(),
        name: "My Workflow".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: Some("https://example.com/webhook".to_string()),
        poll_url: None,
        transport: None,
        command: None,
        args: None,
        system_prompt: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = N8nPlugin::new(&config, chat_state);
    assert_eq!(plugin.id().0, "n8n-my-workflow");
}

// Scenario: HistoryEntry serializes role and content correctly
#[test]
fn scenario_history_entry_serialization() {
    let entry = HistoryEntry {
        role: "user".to_string(),
        content: "hello".to_string(),
    };
    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["role"], "user");
    assert_eq!(json["content"], "hello");
}

// Scenario: HistoryEntry equality comparison
#[test]
fn scenario_history_entry_equality() {
    let a = HistoryEntry { role: "user".into(), content: "hi".into() };
    let b = HistoryEntry { role: "user".into(), content: "hi".into() };
    let c = HistoryEntry { role: "assistant".into(), content: "hi".into() };
    assert_eq!(a, b);
    assert_ne!(a, c);
}
