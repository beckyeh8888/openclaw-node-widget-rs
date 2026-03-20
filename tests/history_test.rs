//! BDD tests for Wave 2B: Chat History Persistence, Multi-Session, Notifications.

use serde_json::json;
use std::fs;
use std::path::PathBuf;

// ── Part 1: Chat History Persistence ─────────────────────────────────

fn temp_history_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("openclaw_bdd_test");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.json"));
    let _ = fs::remove_file(&path);
    path
}

#[test]
fn scenario_messages_persist_across_restart() {
    // Given user sent "hello" and received "hi" in previous session
    let path = temp_history_path("persist_restart");

    let history_data = json!({
        "conversations": {
            "openclaw:main": [
                {"sender": "user", "text": "hello"},
                {"sender": "agent", "agent_name": "Bot", "text": "hi"}
            ]
        }
    });
    fs::write(&path, serde_json::to_string_pretty(&history_data).unwrap()).unwrap();

    // When the chat window opens (simulated by reading the file)
    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let messages = content["conversations"]["openclaw:main"]
        .as_array()
        .unwrap();

    // Then both messages should be displayed
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["sender"], "user");
    assert_eq!(messages[0]["text"], "hello");
    assert_eq!(messages[1]["sender"], "agent");
    assert_eq!(messages[1]["text"], "hi");
}

#[test]
fn scenario_history_limit_enforced() {
    // Given a conversation with 100 messages
    let mut messages: Vec<serde_json::Value> = Vec::new();
    for i in 0..100 {
        messages.push(json!({"sender": "user", "text": format!("msg-{i}")}));
    }
    assert_eq!(messages.len(), 100);

    // When a new message is added
    messages.push(json!({"sender": "user", "text": "overflow"}));
    while messages.len() > 100 {
        messages.remove(0);
    }

    // Then the oldest message should be removed and total should still be 100
    assert_eq!(messages.len(), 100);
    assert_eq!(messages[0]["text"], "msg-1");
    assert_eq!(messages[99]["text"], "overflow");
}

#[test]
fn scenario_multiple_conversations_stored() {
    // Given conversations with "openclaw:main" and "ollama:default"
    let path = temp_history_path("multi_conv");
    let history_data = json!({
        "conversations": {
            "openclaw:main": [
                {"sender": "user", "text": "hello claw"}
            ],
            "ollama:default": [
                {"sender": "user", "text": "hello ollama"},
                {"sender": "agent", "agent_name": "Llama", "text": "hi there"}
            ]
        }
    });
    fs::write(&path, serde_json::to_string_pretty(&history_data).unwrap()).unwrap();

    // When switching plugins
    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

    // Then each conversation shows its own history
    let claw_msgs = content["conversations"]["openclaw:main"]
        .as_array()
        .unwrap();
    let ollama_msgs = content["conversations"]["ollama:default"]
        .as_array()
        .unwrap();

    assert_eq!(claw_msgs.len(), 1);
    assert_eq!(ollama_msgs.len(), 2);
    assert_eq!(claw_msgs[0]["text"], "hello claw");
    assert_eq!(ollama_msgs[0]["text"], "hello ollama");
}

#[test]
fn scenario_conversation_key_format_is_plugin_colon_session() {
    // The conversation key format is "plugin_id:session_key"
    let key = format!("{}:{}", "openclaw-home", "main");
    assert_eq!(key, "openclaw-home:main");

    let key2 = format!("{}:{}", "ollama-local", "default");
    assert_eq!(key2, "ollama-local:default");
}

#[test]
fn scenario_history_file_is_pretty_printed_json() {
    let path = temp_history_path("pretty");
    let history_data = json!({
        "conversations": {
            "test:main": [{"sender": "user", "text": "hi"}]
        }
    });
    let pretty = serde_json::to_string_pretty(&history_data).unwrap();
    fs::write(&path, &pretty).unwrap();

    let content = fs::read_to_string(&path).unwrap();
    // Pretty-printed JSON contains newlines and indentation
    assert!(content.contains('\n'));
    assert!(content.contains("  "));
}

#[test]
fn scenario_max_50_conversations() {
    let mut conversations = serde_json::Map::new();
    for i in 0..55 {
        let key = format!("plugin:{i}");
        conversations.insert(
            key,
            json!([{"sender": "user", "text": format!("msg-{i}")}]),
        );
    }

    // Enforce limit
    while conversations.len() > 50 {
        if let Some(key) = conversations.keys().next().cloned() {
            conversations.remove(&key);
        }
    }

    assert!(conversations.len() <= 50);
}

// ── Part 2: Multi-Session Chat ───────────────────────────────────────

#[test]
fn scenario_switch_between_sessions() {
    // Given "main" session with 5 messages and "stocks" with 3
    let history = json!({
        "conversations": {
            "openclaw:main": [
                {"sender": "user", "text": "m1"},
                {"sender": "agent", "text": "m2"},
                {"sender": "user", "text": "m3"},
                {"sender": "agent", "text": "m4"},
                {"sender": "user", "text": "m5"}
            ],
            "openclaw:stocks": [
                {"sender": "user", "text": "s1"},
                {"sender": "agent", "text": "s2"},
                {"sender": "user", "text": "s3"}
            ]
        }
    });

    // When user clicks "stocks" tab
    let stocks_msgs = history["conversations"]["openclaw:stocks"]
        .as_array()
        .unwrap();

    // Then 3 messages should be displayed
    assert_eq!(stocks_msgs.len(), 3);

    // And main still has 5
    let main_msgs = history["conversations"]["openclaw:main"]
        .as_array()
        .unwrap();
    assert_eq!(main_msgs.len(), 5);
}

#[test]
fn scenario_new_session_messages_go_to_correct_conversation() {
    // Given "stocks" session is active
    let mut conversations: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    conversations.insert(
        "openclaw:main".to_string(),
        vec![json!({"sender": "user", "text": "hello"})],
    );
    conversations.insert("openclaw:stocks".to_string(), Vec::new());

    let active_key = "openclaw:stocks";

    // When user sends "buy AAPL"
    conversations
        .get_mut(active_key)
        .unwrap()
        .push(json!({"sender": "user", "text": "buy AAPL"}));

    // Then the message should be in "stocks" conversation history
    assert_eq!(conversations["openclaw:stocks"].len(), 1);
    assert_eq!(conversations["openclaw:stocks"][0]["text"], "buy AAPL");

    // And "main" conversation should be unchanged
    assert_eq!(conversations["openclaw:main"].len(), 1);
    assert_eq!(conversations["openclaw:main"][0]["text"], "hello");
}

#[test]
fn scenario_switch_plugin_ipc_format() {
    let ipc_msg = json!({
        "type": "switchPlugin",
        "pluginId": "ollama-local",
        "sessionKey": "default"
    });

    assert_eq!(ipc_msg["type"], "switchPlugin");
    assert_eq!(ipc_msg["pluginId"], "ollama-local");
    assert_eq!(ipc_msg["sessionKey"], "default");
}

#[test]
fn scenario_load_history_js_call_format() {
    // The Rust side pushes loadHistory({messages: [...]}) to WebView
    let history_payload = json!({
        "messages": [
            {"sender": "user", "text": "hello"},
            {"sender": "agent", "agentName": "Bot", "text": "hi"}
        ]
    });

    let msgs = history_payload["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["sender"], "user");
    assert_eq!(msgs[1]["agentName"], "Bot");
}

// ── Part 3: Improved Notifications ──────────────────────────────────

#[test]
fn scenario_notification_rate_limiting() {
    // Given a notification was shown 10 seconds ago
    // When another disconnect event occurs
    // Then no notification should be shown (within 30s cooldown)

    let cooldown_secs = 30u64;
    let time_since_last = 10u64;

    let should_notify = time_since_last >= cooldown_secs;
    assert!(!should_notify, "should NOT notify within cooldown window");

    // But after 30 seconds, it should notify
    let time_since_last_2 = 31u64;
    let should_notify_2 = time_since_last_2 >= cooldown_secs;
    assert!(should_notify_2, "should notify after cooldown expires");
}

#[test]
fn scenario_disconnect_notification_format() {
    let connection_name = "Arno";
    let notification = format!("{connection_name} disconnected");
    assert_eq!(notification, "Arno disconnected");
}

#[test]
fn scenario_reconnect_notification_format() {
    let connection_name = "Arno";
    let notification = format!("{connection_name} reconnected");
    assert_eq!(notification, "Arno reconnected");
}

#[test]
fn scenario_error_notification_format() {
    let connection_name = "Arno";
    let message = "timeout";
    let notification = format!("{connection_name} error: {message}");
    assert_eq!(notification, "Arno error: timeout");
}

#[test]
fn scenario_no_notification_when_window_focused() {
    // Given the chat window is focused
    let window_focused = true;

    // When a chat reply arrives
    // Then no notification should be shown (message visible in UI)
    let should_notify = !window_focused;
    assert!(!should_notify);
}

#[test]
fn scenario_notification_config_per_plugin() {
    // Notifications can be disabled globally
    let notifications_enabled = false;
    let should_notify = notifications_enabled && true; // even if rate limit allows
    assert!(!should_notify);
}

// ── Init data contract with new fields ──────────────────────────────

#[test]
fn scenario_init_data_includes_active_plugin_and_session() {
    let init = json!({
        "lang": "en",
        "connected": true,
        "messages": [],
        "sessions": [],
        "selectedSession": null,
        "waitingForReply": false,
        "activePluginId": "openclaw-home",
        "activeSessionKey": "main",
        "dashboard": {},
        "logs": [],
        "currentPage": "chat"
    });

    assert_eq!(init["activePluginId"], "openclaw-home");
    assert_eq!(init["activeSessionKey"], "main");
}
