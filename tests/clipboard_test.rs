//! BDD-style tests for Clipboard Paste Image feature.
//!
//! The clipboard paste handler lives entirely in the JS/WebView layer
//! (chat_ui.html), so these tests validate the Rust-side IPC contract:
//! attachments arriving via the "send" message are correctly parsed and
//! forwarded through the plugin command channel.

use openclaw_node_widget_rs::chat::{handle_ipc_message, ChatState};
use openclaw_node_widget_rs::plugin::PluginCommand;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

// ── Feature: Clipboard Image Paste ──────────────────────────────────

// Scenario: Ctrl+V with image data creates attachment
//   Given the user pastes an image from the clipboard
//   When the JS converts it to base64 and sends via IPC
//   Then the attachment should be forwarded through the plugin command
#[test]
fn scenario_paste_image_creates_attachment() {
    let state = Arc::new(Mutex::new(ChatState::new()));
    {
        let mut s = state.lock().unwrap();
        s.connected = true;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<PluginCommand>();
    let body = serde_json::json!({
        "type": "send",
        "message": "check this image",
        "attachments": [{
            "data": "iVBORw0KGgoAAAANSUhEUg==",
            "filename": "clipboard.png",
            "mimeType": "image/png"
        }]
    })
    .to_string();
    handle_ipc_message(&body, &tx, &state);

    let cmd = rx.try_recv().expect("should receive a command");
    match cmd {
        PluginCommand::SendChat {
            message,
            attachments,
            ..
        } => {
            assert_eq!(message, "check this image");
            let atts = attachments.expect("should have attachments");
            assert_eq!(atts.len(), 1);
            assert_eq!(atts[0].filename, "clipboard.png");
            assert_eq!(atts[0].mime_type, "image/png");
            assert_eq!(atts[0].data, "iVBORw0KGgoAAAANSUhEUg==");
        }
        _ => panic!("expected SendChat command"),
    }
}

// Scenario: Attachment sent with next message
//   Given a pending image attachment and a text message
//   When the user sends the message
//   Then both text and attachment are included in the command
#[test]
fn scenario_attachment_sent_with_message() {
    let state = Arc::new(Mutex::new(ChatState::new()));
    {
        let mut s = state.lock().unwrap();
        s.connected = true;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<PluginCommand>();
    let body = serde_json::json!({
        "type": "send",
        "message": "here is a screenshot",
        "attachments": [{
            "data": "base64data",
            "filename": "screen.jpg",
            "mimeType": "image/jpeg"
        }]
    })
    .to_string();
    handle_ipc_message(&body, &tx, &state);
    let cmd = rx.try_recv().unwrap();
    match cmd {
        PluginCommand::SendChat {
            message,
            attachments,
            ..
        } => {
            assert_eq!(message, "here is a screenshot");
            assert!(attachments.is_some());
            assert_eq!(attachments.unwrap()[0].mime_type, "image/jpeg");
        }
        _ => panic!("expected SendChat"),
    }
}

// Scenario: Non-image paste ignored (text passes through)
//   Given the user sends a regular text message with no attachments
//   When the IPC message arrives
//   Then no attachment is included
#[test]
fn scenario_text_only_no_attachment() {
    let state = Arc::new(Mutex::new(ChatState::new()));
    {
        let mut s = state.lock().unwrap();
        s.connected = true;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<PluginCommand>();
    let body = serde_json::json!({
        "type": "send",
        "message": "hello world"
    })
    .to_string();
    handle_ipc_message(&body, &tx, &state);
    let cmd = rx.try_recv().unwrap();
    match cmd {
        PluginCommand::SendChat {
            message,
            attachments,
            ..
        } => {
            assert_eq!(message, "hello world");
            assert!(attachments.is_none());
        }
        _ => panic!("expected SendChat"),
    }
}

// Scenario: Remove button clears pending attachment
//   Given no attachment
//   When only text is sent
//   Then there is no attachment in the command
#[test]
fn scenario_empty_attachments_array_treated_as_no_attachment() {
    let state = Arc::new(Mutex::new(ChatState::new()));
    {
        let mut s = state.lock().unwrap();
        s.connected = true;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<PluginCommand>();
    let body = serde_json::json!({
        "type": "send",
        "message": "just text",
        "attachments": []
    })
    .to_string();
    handle_ipc_message(&body, &tx, &state);
    // Empty attachments should not block the message
    let cmd = rx.try_recv().unwrap();
    match cmd {
        PluginCommand::SendChat {
            message,
            attachments,
            ..
        } => {
            assert_eq!(message, "just text");
            // Empty vec is parsed but equivalent to no meaningful attachment
            if let Some(ref atts) = attachments {
                assert!(atts.is_empty());
            }
        }
        _ => panic!("expected SendChat"),
    }
}

// Scenario: Image-only message (no text) blocked
//   Given the user has an image attached but typed no text
//   When they try to send
//   Then the message is still sent (has_attachments = true)
#[test]
fn scenario_image_only_message_sent() {
    let state = Arc::new(Mutex::new(ChatState::new()));
    {
        let mut s = state.lock().unwrap();
        s.connected = true;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<PluginCommand>();
    let body = serde_json::json!({
        "type": "send",
        "message": "",
        "attachments": [{
            "data": "abc123",
            "filename": "clipboard.png",
            "mimeType": "image/png"
        }]
    })
    .to_string();
    handle_ipc_message(&body, &tx, &state);
    let cmd = rx.try_recv().unwrap();
    match cmd {
        PluginCommand::SendChat { attachments, .. } => {
            let atts = attachments.expect("should have attachments");
            assert_eq!(atts.len(), 1);
        }
        _ => panic!("expected SendChat"),
    }
}
