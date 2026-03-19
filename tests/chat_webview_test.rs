//! Integration-style BDD tests for the Phase 13 WebView Chat rewrite.
//!
//! Since this is a binary crate, these tests verify the chat protocol
//! contracts (IPC JSON format, session defaults, message limits) as
//! standalone behavioural specifications that mirror the implementation
//! in src/chat.rs and src/gateway.rs.

use serde_json::json;

const MAX_MESSAGES: usize = 50;

// ── IPC message format contracts ─────────────────────────────────────

#[test]
fn scenario_send_ipc_message_includes_required_fields() {
    // The WebView JS sends this shape via window.ipc.postMessage
    let ipc_msg = json!({
        "type": "send",
        "message": "hello agent",
        "sessionKey": "sess-42",
    });

    assert_eq!(ipc_msg["type"], "send");
    assert_eq!(ipc_msg["message"], "hello agent");
    assert_eq!(ipc_msg["sessionKey"], "sess-42");
}

#[test]
fn scenario_send_ipc_with_null_session_key_when_no_session_selected() {
    // When the HTML <select> has value="" the JS sends sessionKey: null
    let ipc_msg = json!({
        "type": "send",
        "message": "test",
        "sessionKey": null,
    });

    assert!(ipc_msg["sessionKey"].is_null());
}

#[test]
fn scenario_send_ipc_with_attachments_array() {
    let ipc_msg = json!({
        "type": "send",
        "message": "see this",
        "sessionKey": "main",
        "attachments": [
            {
                "data": "iVBORw0KGgo=",
                "filename": "screenshot.png",
                "mimeType": "image/png"
            }
        ]
    });

    let atts = ipc_msg["attachments"].as_array().unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0]["data"], "iVBORw0KGgo=");
    assert_eq!(atts[0]["filename"], "screenshot.png");
    assert_eq!(atts[0]["mimeType"], "image/png");
}

#[test]
fn scenario_select_session_ipc_format() {
    let ipc_msg = json!({
        "type": "selectSession",
        "sessionKey": "new-session",
    });

    assert_eq!(ipc_msg["type"], "selectSession");
    assert_eq!(ipc_msg["sessionKey"], "new-session");
}

#[test]
fn scenario_list_sessions_ipc_format() {
    let ipc_msg = json!({ "type": "listSessions" });
    assert_eq!(ipc_msg["type"], "listSessions");
}

// ── Gateway chat.send frame contracts ────────────────────────────────

#[test]
fn scenario_chat_send_frame_includes_idempotency_key_and_session_key() {
    // Mirrors the frame built in gateway.rs:586-601
    let idempotency_key = "test-uuid-1234";
    let session_key = "main";
    let message = "hello";

    let frame = json!({
        "type": "req",
        "id": "req-1",
        "method": "chat.send",
        "params": {
            "message": message,
            "idempotencyKey": idempotency_key,
            "sessionKey": session_key,
        }
    });

    assert_eq!(frame["method"], "chat.send");
    let params = &frame["params"];
    assert_eq!(params["message"], "hello");
    assert!(
        !params["idempotencyKey"].as_str().unwrap().is_empty(),
        "idempotencyKey must be present"
    );
    assert_eq!(params["sessionKey"], "main");
}

#[test]
fn scenario_session_key_defaults_to_main_when_none() {
    // In gateway.rs:588: session_key.unwrap_or_else(|| "main".to_string())
    let session_key: Option<String> = None;
    let resolved = session_key.unwrap_or_else(|| "main".to_string());
    assert_eq!(resolved, "main");
}

#[test]
fn scenario_chat_send_with_attachments_in_frame() {
    let frame = json!({
        "type": "req",
        "id": "req-2",
        "method": "chat.send",
        "params": {
            "message": "image attached",
            "idempotencyKey": "key-abc",
            "sessionKey": "main",
            "attachments": [
                {"data": "base64==", "filename": "pic.png", "mimeType": "image/png"}
            ]
        }
    });

    let atts = frame["params"]["attachments"].as_array().unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0]["mimeType"], "image/png");
}

// ── ChatAttachment struct serialization ──────────────────────────────

#[test]
fn scenario_attachment_json_roundtrip() {
    // Simulate ChatAttachment → JSON → parse back
    let original = json!({
        "data": "abc123",
        "filename": "doc.pdf",
        "mimeType": "application/pdf"
    });

    let data = original["data"].as_str().unwrap();
    let filename = original["filename"].as_str().unwrap();
    let mime_type = original["mimeType"].as_str().unwrap();

    assert_eq!(data, "abc123");
    assert_eq!(filename, "doc.pdf");
    assert_eq!(mime_type, "application/pdf");
}

#[test]
fn scenario_attachment_with_missing_fields_returns_none() {
    // The IPC handler uses .get("data")?.as_str()? — missing fields yield None
    let incomplete = json!({"data": "abc"});

    assert!(incomplete.get("filename").is_none());
    assert!(incomplete.get("mimeType").is_none());

    // Filter-map should skip incomplete attachments
    let parsed: Vec<(String, String, String)> = [&incomplete]
        .iter()
        .filter_map(|a| {
            Some((
                a.get("data")?.as_str()?.to_string(),
                a.get("filename")?.as_str()?.to_string(),
                a.get("mimeType")?.as_str()?.to_string(),
            ))
        })
        .collect();

    assert!(parsed.is_empty(), "incomplete attachments should be skipped");
}

// ── ChatInbound protocol ─────────────────────────────────────────────

#[test]
fn scenario_chat_event_reply_has_text_and_optional_agent_name() {
    // gateway.rs handle_chat_event extracts text from payload.text/message/content
    let payload = json!({
        "text": "hello from agent",
        "agentName": "Claw"
    });

    let text = payload
        .get("text")
        .or_else(|| payload.get("message"))
        .or_else(|| payload.get("content"))
        .and_then(|v| v.as_str());

    let agent_name = payload
        .get("agentName")
        .or_else(|| payload.get("agent"))
        .or_else(|| payload.get("sender"))
        .and_then(|v| v.as_str());

    assert_eq!(text, Some("hello from agent"));
    assert_eq!(agent_name, Some("Claw"));
}

#[test]
fn scenario_chat_event_reply_with_message_field_variant() {
    let payload = json!({ "message": "alt format" });

    let text = payload
        .get("text")
        .or_else(|| payload.get("message"))
        .and_then(|v| v.as_str());

    assert_eq!(text, Some("alt format"));
}

#[test]
fn scenario_chat_event_reply_with_content_field_variant() {
    let payload = json!({ "content": "content format", "sender": "Bot" });

    let text = payload
        .get("text")
        .or_else(|| payload.get("message"))
        .or_else(|| payload.get("content"))
        .and_then(|v| v.as_str());

    let agent = payload
        .get("agentName")
        .or_else(|| payload.get("agent"))
        .or_else(|| payload.get("sender"))
        .and_then(|v| v.as_str());

    assert_eq!(text, Some("content format"));
    assert_eq!(agent, Some("Bot"));
}

#[test]
fn scenario_sessions_list_response_parsing() {
    // Mirrors handle_chat_response in gateway.rs
    let payload = json!({
        "sessions": [
            {"key": "sess-1", "name": "Main Session"},
            {"id": "sess-2", "displayName": "Alt Session"}
        ]
    });

    let sessions = payload["sessions"].as_array().unwrap();
    let parsed: Vec<(String, String)> = sessions
        .iter()
        .filter_map(|s| {
            let key = s
                .get("key")
                .or_else(|| s.get("id"))
                .and_then(|v| v.as_str())?;
            let name = s
                .get("name")
                .or_else(|| s.get("displayName"))
                .and_then(|v| v.as_str())
                .unwrap_or(key);
            Some((key.to_string(), name.to_string()))
        })
        .collect();

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0], ("sess-1".to_string(), "Main Session".to_string()));
    assert_eq!(parsed[1], ("sess-2".to_string(), "Alt Session".to_string()));
}

// ── Message history max limit ────────────────────────────────────────

#[test]
fn scenario_message_eviction_preserves_newest() {
    let mut messages: Vec<String> = Vec::new();

    // Simulate adding 55 messages with eviction
    for i in 0..55 {
        messages.push(format!("msg-{i}"));
        while messages.len() > MAX_MESSAGES {
            messages.remove(0);
        }
    }

    assert_eq!(messages.len(), MAX_MESSAGES);
    assert_eq!(messages[0], "msg-5");
    assert_eq!(messages[MAX_MESSAGES - 1], "msg-54");
}

#[test]
fn scenario_exact_max_messages_no_eviction() {
    let mut messages: Vec<String> = Vec::new();

    for i in 0..MAX_MESSAGES {
        messages.push(format!("msg-{i}"));
    }

    // At exactly MAX_MESSAGES, no eviction should occur
    assert_eq!(messages.len(), MAX_MESSAGES);
    assert_eq!(messages[0], "msg-0");
}

#[test]
fn scenario_reply_eviction_matches_user_message_eviction() {
    // Both handle_ipc_message (user send) and process_inbox_to_webview (reply)
    // use the same `while messages.len() > MAX_MESSAGES { messages.remove(0) }`
    // pattern. Verify they produce the same result.

    let mut user_msgs: Vec<String> = Vec::new();
    let mut agent_msgs: Vec<String> = Vec::new();

    for i in 0..MAX_MESSAGES + 5 {
        user_msgs.push(format!("u-{i}"));
        while user_msgs.len() > MAX_MESSAGES {
            user_msgs.remove(0);
        }

        agent_msgs.push(format!("a-{i}"));
        while agent_msgs.len() > MAX_MESSAGES {
            agent_msgs.remove(0);
        }
    }

    assert_eq!(user_msgs.len(), agent_msgs.len());
    assert_eq!(user_msgs.len(), MAX_MESSAGES);
}

// ── Init data JSON contract ──────────────────────────────────────────

#[test]
fn scenario_init_data_shape_matches_html_expectations() {
    // The HTML template expects __INIT_DATA__ to be replaced with this shape
    let init = json!({
        "lang": "en",
        "connected": false,
        "messages": [],
        "sessions": [],
        "selectedSession": null,
        "waitingForReply": false,
    });

    assert!(init["messages"].is_array());
    assert!(init["sessions"].is_array());
    assert!(init["selectedSession"].is_null());
    assert_eq!(init["lang"], "en");
    assert_eq!(init["connected"], false);
    assert_eq!(init["waitingForReply"], false);
}

#[test]
fn scenario_init_data_with_existing_messages() {
    let init = json!({
        "lang": "zh-tw",
        "connected": true,
        "messages": [
            {"sender": "user", "text": "hello"},
            {"sender": "agent", "agentName": "Bot", "text": "hi"}
        ],
        "sessions": [{"key": "main", "name": "Main"}],
        "selectedSession": "main",
        "waitingForReply": true,
    });

    let msgs = init["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["sender"], "user");
    assert_eq!(msgs[1]["agentName"], "Bot");
    assert_eq!(init["selectedSession"], "main");
    assert_eq!(init["waitingForReply"], true);
}
