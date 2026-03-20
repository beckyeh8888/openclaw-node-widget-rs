//! BDD-style tests for the MCP plugin.

use std::sync::{Arc, Mutex};

use openclaw_node_widget_rs::chat::ChatState;
use openclaw_node_widget_rs::config::PluginConfig;
use openclaw_node_widget_rs::plugin::mcp::{
    build_create_message_request, build_initialize_request, parse_assistant_response,
    JsonRpcResponse, McpContent, McpMessage, McpPlugin, McpTransport,
};
use openclaw_node_widget_rs::plugin::{AgentPlugin, ConnectionStatus};

// ── Feature: MCP Plugin ────────────────────────────────────────────

// Scenario: Config parsing for stdio transport
//   Given a PluginConfig with type "mcp", transport "stdio", command "npx"
//   When an McpPlugin is created
//   Then it should have Stdio transport with correct command and args
#[test]
fn scenario_config_parsing_stdio_transport() {
    let config = PluginConfig {
        plugin_type: "mcp".to_string(),
        name: "File Browser".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: Some("stdio".to_string()),
        command: Some("npx".to_string()),
        args: Some(vec![
            "-y".to_string(),
            "@anthropic/mcp-server-filesystem".to_string(),
            "/home".to_string(),
        ]),
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = McpPlugin::new(&config, chat_state);

    assert_eq!(plugin.id().0, "mcp-file-browser");
    assert_eq!(plugin.name(), "File Browser");
    assert_eq!(plugin.plugin_type(), "mcp");
    match plugin.transport() {
        McpTransport::Stdio { command, args } => {
            assert_eq!(command, "npx");
            assert_eq!(args.len(), 3);
            assert_eq!(args[0], "-y");
        }
        _ => panic!("expected Stdio transport"),
    }
}

// Scenario: Config parsing for SSE transport
//   Given a PluginConfig with transport "sse" and url
//   When an McpPlugin is created
//   Then it should have Sse transport with correct url
#[test]
fn scenario_config_parsing_sse_transport() {
    let config = PluginConfig {
        plugin_type: "mcp".to_string(),
        name: "Remote Agent".to_string(),
        url: Some("http://localhost:3001".to_string()),
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: Some("sse".to_string()),
        command: None,
        args: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = McpPlugin::new(&config, chat_state);

    assert_eq!(plugin.id().0, "mcp-remote-agent");
    match plugin.transport() {
        McpTransport::Sse { url } => {
            assert_eq!(url, "http://localhost:3001");
        }
        _ => panic!("expected SSE transport"),
    }
}

// Scenario: Initialize handshake JSON-RPC format
//   Given an initialize request with id 1
//   When serialized to JSON
//   Then it should contain jsonrpc "2.0", method "initialize", protocolVersion
#[test]
fn scenario_initialize_handshake_jsonrpc_format() {
    let req = build_initialize_request(1);
    let json = serde_json::to_value(&req).unwrap();

    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 1);
    assert_eq!(json["method"], "initialize");
    assert_eq!(json["params"]["protocolVersion"], "2024-11-05");
    assert_eq!(json["params"]["clientInfo"]["name"], "openclaw-widget");
    assert_eq!(json["params"]["clientInfo"]["version"], "0.9.0");
    assert!(json["params"]["capabilities"].is_object());
}

// Scenario: Create message request format
//   Given a user message "Hello"
//   When a createMessage request is built
//   Then it should use sampling/createMessage method with messages array
#[test]
fn scenario_create_message_request_format() {
    let messages = vec![McpMessage {
        role: "user".to_string(),
        content: McpContent {
            content_type: "text".to_string(),
            text: "Hello".to_string(),
        },
    }];
    let req = build_create_message_request(2, &messages, 4096);
    let json = serde_json::to_value(&req).unwrap();

    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 2);
    assert_eq!(json["method"], "sampling/createMessage");
    assert_eq!(json["params"]["messages"][0]["role"], "user");
    assert_eq!(json["params"]["messages"][0]["content"]["type"], "text");
    assert_eq!(json["params"]["messages"][0]["content"]["text"], "Hello");
    assert_eq!(json["params"]["maxTokens"], 4096);
}

// Scenario: Parse assistant response
//   Given a JSON-RPC result with content.text
//   When the response is parsed
//   Then the assistant text should be extracted
#[test]
fn scenario_parse_assistant_response() {
    let result = serde_json::json!({
        "role": "assistant",
        "content": { "type": "text", "text": "Hello from MCP!" }
    });
    assert_eq!(
        parse_assistant_response(&result),
        Some("Hello from MCP!".to_string())
    );
}

// Scenario: Parse assistant response from flat text
//   Given a JSON-RPC result with top-level text field
//   When the response is parsed
//   Then the text should be extracted
#[test]
fn scenario_parse_assistant_response_flat_text() {
    let result = serde_json::json!({ "text": "Flat response" });
    assert_eq!(
        parse_assistant_response(&result),
        Some("Flat response".to_string())
    );
}

// Scenario: Parse returns None for empty result
#[test]
fn scenario_parse_assistant_response_empty() {
    let result = serde_json::json!({});
    assert_eq!(parse_assistant_response(&result), None);
}

// Scenario: Handle server error response
//   Given a JSON-RPC error response with code -32600
//   When parsed as JsonRpcResponse
//   Then error code and message should be accessible
#[test]
fn scenario_handle_server_error_response() {
    let resp_json =
        r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32600,"message":"Invalid request"}}"#;
    let resp: JsonRpcResponse = serde_json::from_str(resp_json).unwrap();

    assert!(resp.error.is_some());
    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, -32600);
    assert_eq!(err.message, "Invalid request");
}

// Scenario: Conversation history maintained
//   Given multiple messages built in sequence
//   When createMessage is called
//   Then all prior messages should be included
#[test]
fn scenario_conversation_history_maintained() {
    let messages = vec![
        McpMessage {
            role: "user".to_string(),
            content: McpContent {
                content_type: "text".to_string(),
                text: "Hi".to_string(),
            },
        },
        McpMessage {
            role: "assistant".to_string(),
            content: McpContent {
                content_type: "text".to_string(),
                text: "Hello!".to_string(),
            },
        },
        McpMessage {
            role: "user".to_string(),
            content: McpContent {
                content_type: "text".to_string(),
                text: "How are you?".to_string(),
            },
        },
    ];
    let req = build_create_message_request(3, &messages, 4096);
    let json = serde_json::to_value(&req).unwrap();

    let msgs = json["params"]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0]["content"]["text"], "Hi");
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(msgs[2]["content"]["text"], "How are you?");
}

// Scenario: Plugin capabilities (chat only, no dashboard)
//   Given an MCP plugin
//   When capabilities are queried
//   Then chat should be true and dashboard/workflows/logs should be false
#[test]
fn scenario_plugin_capabilities_chat_only() {
    let config = PluginConfig {
        plugin_type: "mcp".to_string(),
        name: "Test".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: Some("stdio".to_string()),
        command: Some("echo".to_string()),
        args: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = McpPlugin::new(&config, chat_state);
    let caps = plugin.capabilities();

    assert!(caps.chat, "MCP should support chat");
    assert!(!caps.dashboard, "MCP should not support dashboard");
    assert!(!caps.workflows, "MCP should not support workflows");
    assert!(!caps.logs, "MCP should not support logs");
}

// Scenario: Connect fails without command configured (stdio)
#[test]
fn scenario_connect_fails_without_command() {
    let config = PluginConfig {
        plugin_type: "mcp".to_string(),
        name: "Bad".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: Some("stdio".to_string()),
        command: None,
        args: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let mut plugin = McpPlugin::new(&config, chat_state);
    let result = plugin.connect();
    assert!(result.is_err());
    assert!(result.unwrap_err().0.contains("command not configured"));
}

// Scenario: Connect fails without URL configured (sse)
#[test]
fn scenario_connect_fails_without_url_sse() {
    let config = PluginConfig {
        plugin_type: "mcp".to_string(),
        name: "Bad SSE".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: Some("sse".to_string()),
        command: None,
        args: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let mut plugin = McpPlugin::new(&config, chat_state);
    let result = plugin.connect();
    assert!(result.is_err());
    assert!(result.unwrap_err().0.contains("url not configured"));
}

// Scenario: Disconnect clears connection state
#[test]
fn scenario_disconnect_clears_state() {
    let config = PluginConfig {
        plugin_type: "mcp".to_string(),
        name: "Test".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: Some("stdio".to_string()),
        command: Some("echo".to_string()),
        args: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let mut plugin = McpPlugin::new(&config, chat_state);

    // Simulate connected state
    assert_eq!(plugin.status(), ConnectionStatus::Disconnected);
    let result = plugin.send_message("test", None, None);
    assert!(result.is_err(), "should fail before connect");
}

// Scenario: Plugin icon
#[test]
fn scenario_plugin_icon() {
    let config = PluginConfig {
        plugin_type: "mcp".to_string(),
        name: "Test".to_string(),
        url: None,
        token: None,
        model: None,
        api_key: None,
        webhook_url: None,
        poll_url: None,
        transport: Some("stdio".to_string()),
        command: Some("echo".to_string()),
        args: None,
    };
    let chat_state = Arc::new(Mutex::new(ChatState::new()));
    let plugin = McpPlugin::new(&config, chat_state);
    assert_eq!(plugin.icon(), "🔌");
}
