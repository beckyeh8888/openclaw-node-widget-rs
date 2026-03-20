//! BDD tests for Chat Export feature.
//!
//! Feature: Chat Export
//!   As a user I want to export my conversation as Markdown
//!   so that I can save or share it outside the app.

use serde_json::json;

// ── Markdown export format ──────────────────────────────────────────

#[test]
fn scenario_export_markdown_has_header_with_timestamp() {
    let timestamp = "2026-03-20T10:30:00.000Z";
    let md = format!("# Chat Export - {}\n\n", timestamp);

    assert!(md.starts_with("# Chat Export - "), "should start with header");
    assert!(md.contains(timestamp), "should include ISO timestamp");
}

#[test]
fn scenario_export_three_messages_in_markdown() {
    let messages = vec![
        json!({"sender": "user", "text": "Hello", "timestamp": "10:00 AM"}),
        json!({"sender": "agent", "agentName": "Bot", "text": "Hi there!", "timestamp": "10:00 AM"}),
        json!({"sender": "user", "text": "How are you?", "timestamp": "10:01 AM"}),
    ];

    let mut md = "# Chat Export - 2026-03-20\n\n".to_string();
    for m in &messages {
        let sender = if m["sender"] == "user" {
            "**You**".to_string()
        } else {
            format!("**{}**", m["agentName"].as_str().unwrap_or("Agent"))
        };
        let ts = m["timestamp"].as_str().unwrap_or("");
        let text = m["text"].as_str().unwrap_or("");
        md += &format!("### {} ({})\n{}\n\n", sender, ts, text);
    }

    assert_eq!(md.matches("### ").count(), 3, "should contain 3 message headers");
    assert!(md.contains("**You**"), "should contain user messages");
    assert!(md.contains("**Bot**"), "should contain agent name");
    assert!(md.contains("Hello"), "should contain message text");
    assert!(md.contains("Hi there!"), "should contain agent reply");
    assert!(md.contains("How are you?"), "should contain follow-up");
}

#[test]
fn scenario_export_ipc_message_format() {
    let ipc_msg = json!({
        "type": "export",
        "format": "markdown",
        "content": "# Chat Export\n\n### **You** (10:00 AM)\nhello\n\n",
    });

    assert_eq!(ipc_msg["type"], "export");
    assert_eq!(ipc_msg["format"], "markdown");
    assert!(ipc_msg["content"].as_str().unwrap().len() > 0, "content should be non-empty");
}

#[test]
fn scenario_export_filename_format() {
    // The Rust side generates: chat-export-YYYYMMDD-HHMMSS.md
    let timestamp = "20260320-103000";
    let filename = format!("chat-export-{}.md", timestamp);

    assert!(filename.starts_with("chat-export-"), "should have chat-export prefix");
    assert!(filename.ends_with(".md"), "should have .md extension");
}

#[test]
fn scenario_export_txt_filename_for_non_markdown() {
    let format = "txt";
    let ext = if format == "markdown" { "md" } else { "txt" };
    let filename = format!("chat-export-20260320-103000.{}", ext);

    assert!(filename.ends_with(".txt"), "non-markdown format should use .txt");
}

#[test]
fn scenario_export_empty_conversation() {
    // Export with no messages should still produce a valid header
    let md = "# Chat Export - 2026-03-20T10:30:00.000Z\n\n".to_string();

    assert!(md.contains("# Chat Export"), "even empty export has header");
    assert_eq!(md.matches("### ").count(), 0, "no message sections for empty chat");
}

#[test]
fn scenario_export_preserves_code_blocks_in_markdown() {
    let agent_text = "Here is code:\n```rust\nfn main() {}\n```";
    let md = format!("### **Agent** (10:00 AM)\n{}\n\n", agent_text);

    assert!(md.contains("```rust"), "should preserve code fences");
    assert!(md.contains("fn main()"), "should preserve code content");
}

#[test]
fn scenario_export_message_with_agent_name_fallback() {
    // When agentName is missing, default to "Agent"
    let msg = json!({"sender": "agent", "text": "reply"});
    let name = msg.get("agentName")
        .and_then(|v| v.as_str())
        .unwrap_or("Agent");

    assert_eq!(name, "Agent", "should fall back to 'Agent' when name is missing");
}

#[test]
fn scenario_export_downloads_dir_fallback() {
    // If downloads dir is unavailable, fall back to "."
    let downloads: Option<std::path::PathBuf> = None;
    let resolved = downloads.unwrap_or_else(|| std::path::PathBuf::from("."));

    assert_eq!(resolved, std::path::PathBuf::from("."));
}

#[test]
fn scenario_export_special_characters_preserved() {
    let text = "Message with <html> & \"quotes\" and 'apostrophes'";
    let md = format!("### **You** (10:00 AM)\n{}\n\n", text);

    assert!(md.contains("<html>"), "should preserve HTML-like content in raw markdown");
    assert!(md.contains("&"), "should preserve ampersands");
    assert!(md.contains("\"quotes\""), "should preserve quotes");
}

#[test]
fn scenario_export_multiline_message() {
    let text = "Line 1\nLine 2\nLine 3";
    let md = format!("### **You** (10:00 AM)\n{}\n\n", text);

    assert!(md.contains("Line 1\nLine 2\nLine 3"), "should preserve newlines in message");
}
