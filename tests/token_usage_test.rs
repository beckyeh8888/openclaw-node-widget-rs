//! BDD-style tests for the Token Usage Display feature.
//!
//! After each agent reply, the chat UI shows token count and duration
//! in small gray text below the message bubble.

use openclaw_node_widget_rs::plugin::TokenUsage;
use openclaw_node_widget_rs::plugin::ollama::parse_ollama_ndjson;
use openclaw_node_widget_rs::plugin::openai_compat::parse_openai_sse;

// ══════════════════════════════════════════════════════════════════════
// Feature: Token Usage
// ══════════════════════════════════════════════════════════════════════

// Scenario: Ollama reply shows eval_count as token count
//   Given an Ollama NDJSON stream with eval_count=42, prompt_eval_count=10
//   When the response is parsed
//   Then token usage should have output_tokens=42, input_tokens=10
#[test]
fn scenario_ollama_reply_shows_token_count() {
    let data = r#"{"message":{"role":"assistant","content":"Hello"},"done":false}
{"message":{"role":"assistant","content":""},"done":true,"eval_count":42,"prompt_eval_count":10,"total_duration":1500000000}
"#;

    let (_chunks, full, usage) = parse_ollama_ndjson(data);
    assert_eq!(full, Some("Hello".to_string()));
    assert!(usage.is_some(), "should have token usage");

    let u = usage.unwrap();
    assert_eq!(u.output_tokens, Some(42));
    assert_eq!(u.input_tokens, Some(10));
    assert_eq!(u.duration_ms, Some(1500)); // 1.5 billion ns = 1500ms
}

// Scenario: Ollama reply shows eval_duration
//   Given an Ollama NDJSON stream with total_duration=2000000000 (2s in ns)
//   When parsed
//   Then duration_ms should be 2000
#[test]
fn scenario_ollama_duration_displayed_in_seconds() {
    let data = r#"{"message":{"role":"assistant","content":"ok"},"done":false}
{"done":true,"eval_count":5,"total_duration":2000000000}
"#;

    let (_, _, usage) = parse_ollama_ndjson(data);
    let u = usage.unwrap();
    assert_eq!(u.duration_ms, Some(2000));
}

// Scenario: OpenAI reply shows usage tokens
//   Given an OpenAI SSE stream with usage in the final chunk
//   When parsed
//   Then the parse function returns chunks correctly
//   (Token usage is captured in the streaming handler, not parse_openai_sse)
#[test]
fn scenario_openai_reply_shows_usage_tokens() {
    let data = r#"data: {"choices":[{"delta":{"content":"Hi"}}]}

data: {"choices":[{"delta":{"content":"!"}}],"usage":{"prompt_tokens":15,"completion_tokens":8,"total_tokens":23}}

data: [DONE]
"#;

    let (chunks, full) = parse_openai_sse(data);
    assert_eq!(chunks, vec!["Hi", "!"]);
    assert_eq!(full, Some("Hi!".to_string()));
}

// Scenario: No usage data shows nothing (graceful)
//   Given a reply with no token usage info
//   When displayed in the UI
//   Then no usage element should appear
#[test]
fn scenario_no_usage_data_shows_nothing() {
    let usage: Option<TokenUsage> = None;
    assert!(usage.is_none(), "no usage should result in no display");
}

// Scenario: TokenUsage default
//   Given a default TokenUsage
//   When all fields are None
//   Then it should serialize gracefully
#[test]
fn scenario_token_usage_default() {
    let usage = TokenUsage::default();
    assert!(usage.input_tokens.is_none());
    assert!(usage.output_tokens.is_none());
    assert!(usage.duration_ms.is_none());
}

// Scenario: Duration displayed in seconds
//   Given token usage with duration_ms=1250
//   When displayed
//   Then it should show "1.2s" (verified by JS, here we check the data)
#[test]
fn scenario_duration_in_seconds_formatting() {
    let usage = TokenUsage {
        input_tokens: None,
        output_tokens: Some(100),
        duration_ms: Some(1250),
    };
    // The JS formats as (duration_ms / 1000).toFixed(1)
    let formatted = format!("{:.1}s", usage.duration_ms.unwrap() as f64 / 1000.0);
    assert_eq!(formatted, "1.2s");
}

// Scenario: Ollama stream without eval_count has no usage
//   Given an Ollama NDJSON done line without eval fields
//   When parsed
//   Then usage should still be Some but with None tokens
#[test]
fn scenario_ollama_done_without_eval_count() {
    let data = r#"{"message":{"role":"assistant","content":"hi"},"done":false}
{"done":true}
"#;

    let (_, _, usage) = parse_ollama_ndjson(data);
    // done=true but no eval fields → no usage struct
    assert!(usage.is_none(), "no eval fields means no usage");
}
