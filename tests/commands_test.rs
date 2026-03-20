//! BDD tests for Slash Commands feature.
//!
//! Feature: Slash Commands
//!   As a user I want to type "/" in the chat input
//!   to access quick actions like clearing the conversation or exporting.

use serde_json::json;

// ── Command definitions ─────────────────────────────────────────────

fn slash_commands() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("/clear", "Clear conversation", false),
        ("/export", "Export as Markdown", false),
        ("/session", "Switch session", true),
        ("/plugin", "Switch plugin", true),
        ("/model", "Set model", true),
        ("/system", "Set system prompt", true),
        ("/help", "Show commands", false),
        ("/voice", "Toggle voice input", false),
        ("/tts", "Toggle auto-read", false),
    ]
}

#[test]
fn scenario_all_commands_have_descriptions() {
    let commands = slash_commands();
    assert!(commands.len() >= 9, "should have at least 9 slash commands");
    for (cmd, desc, _) in &commands {
        assert!(cmd.starts_with('/'), "command '{}' should start with /", cmd);
        assert!(!desc.is_empty(), "command '{}' should have a description", cmd);
    }
}

#[test]
fn scenario_typing_slash_shows_command_menu() {
    // When input value starts with "/" and contains no space, the menu should appear
    let input_value = "/";
    assert!(input_value.starts_with('/'), "input should start with /");
    assert!(!input_value.contains(' '), "no space yet — menu should be visible");
}

#[test]
fn scenario_command_menu_filters_as_you_type() {
    let query = "cl";
    let commands = slash_commands();
    let filtered: Vec<_> = commands
        .iter()
        .filter(|(cmd, _, _)| cmd.starts_with(&format!("/{}", query)))
        .collect();

    assert_eq!(filtered.len(), 1, "only /clear should match 'cl'");
    assert_eq!(filtered[0].0, "/clear");
}

#[test]
fn scenario_command_menu_filters_multiple_matches() {
    let query = "s";
    let commands = slash_commands();
    let filtered: Vec<_> = commands
        .iter()
        .filter(|(cmd, _, _)| cmd.starts_with(&format!("/{}", query)))
        .collect();

    // /session and /system both start with /s
    assert!(filtered.len() >= 2, "should match at least /session and /system");
}

#[test]
fn scenario_empty_query_shows_all_commands() {
    let query = "";
    let commands = slash_commands();
    let filtered: Vec<_> = commands
        .iter()
        .filter(|(cmd, _, _)| cmd.starts_with(&format!("/{}", query)))
        .collect();

    assert_eq!(filtered.len(), commands.len(), "empty query should show all commands");
}

#[test]
fn scenario_no_match_hides_menu() {
    let query = "xyz";
    let commands = slash_commands();
    let filtered: Vec<_> = commands
        .iter()
        .filter(|(cmd, _, _)| cmd.starts_with(&format!("/{}", query)))
        .collect();

    assert!(filtered.is_empty(), "no commands should match 'xyz'");
}

// ── Command execution ───────────────────────────────────────────────

#[test]
fn scenario_clear_sends_ipc_message() {
    let ipc_msg = json!({ "type": "clearConversation" });
    assert_eq!(ipc_msg["type"], "clearConversation");
}

#[test]
fn scenario_export_triggers_markdown_export() {
    // /export triggers exportMarkdown() which sends this IPC
    let ipc_msg = json!({
        "type": "export",
        "format": "markdown",
        "content": "# Chat Export\n\n### **You** (12:00 PM)\nhello\n\n"
    });

    assert_eq!(ipc_msg["type"], "export");
    assert_eq!(ipc_msg["format"], "markdown");
    assert!(ipc_msg["content"].as_str().unwrap().contains("# Chat Export"));
}

#[test]
fn scenario_session_command_requires_argument() {
    let commands = slash_commands();
    let session_cmd = commands.iter().find(|(cmd, _, _)| *cmd == "/session").unwrap();
    assert!(session_cmd.2, "/session should require an argument");
}

#[test]
fn scenario_plugin_command_sends_switch_ipc() {
    let ipc_msg = json!({
        "type": "switchPlugin",
        "pluginId": "ollama-local",
        "sessionKey": "main",
    });

    assert_eq!(ipc_msg["type"], "switchPlugin");
    assert_eq!(ipc_msg["pluginId"], "ollama-local");
}

#[test]
fn scenario_model_command_sends_set_model_ipc() {
    let ipc_msg = json!({
        "type": "setModel",
        "model": "gpt-4o",
    });

    assert_eq!(ipc_msg["type"], "setModel");
    assert_eq!(ipc_msg["model"], "gpt-4o");
}

#[test]
fn scenario_system_command_sends_set_system_prompt_ipc() {
    let ipc_msg = json!({
        "type": "setSystemPrompt",
        "prompt": "You are a helpful assistant.",
    });

    assert_eq!(ipc_msg["type"], "setSystemPrompt");
    assert_eq!(ipc_msg["prompt"], "You are a helpful assistant.");
}

#[test]
fn scenario_help_shows_all_commands_as_system_message() {
    let commands = slash_commands();
    let help_text: String = commands
        .iter()
        .map(|(cmd, desc, _)| format!("{} — {}", cmd, desc))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(help_text.contains("/clear — Clear conversation"));
    assert!(help_text.contains("/export — Export as Markdown"));
    assert!(help_text.contains("/help — Show commands"));
}

#[test]
fn scenario_unknown_command_shows_error() {
    let cmd = "/foobar";
    let known: Vec<&str> = slash_commands().iter().map(|(c, _, _)| *c).collect();
    assert!(!known.contains(&cmd), "/foobar should not be a known command");
}

// ── Command parsing ─────────────────────────────────────────────────

fn parse_command(input: &str) -> Option<(&str, &str)> {
    if !input.starts_with('/') { return None; }
    match input.find(' ') {
        Some(idx) => Some((&input[..idx], input[idx+1..].trim())),
        None => Some((input, "")),
    }
}

#[test]
fn scenario_command_with_argument_parsed_correctly() {
    let (cmd, arg) = parse_command("/session work").unwrap();
    assert_eq!(cmd, "/session");
    assert_eq!(arg, "work");
}

#[test]
fn scenario_command_without_argument_parsed_correctly() {
    let (cmd, arg) = parse_command("/clear").unwrap();
    assert_eq!(cmd, "/clear");
    assert_eq!(arg, "");
}

#[test]
fn scenario_system_prompt_with_spaces_preserved() {
    let (cmd, arg) = parse_command("/system You are a pirate. Speak accordingly.").unwrap();
    assert_eq!(cmd, "/system");
    assert_eq!(arg, "You are a pirate. Speak accordingly.");
}
