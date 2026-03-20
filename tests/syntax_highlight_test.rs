//! BDD-style tests for Code Syntax Highlighting feature.
//!
//! The syntax highlighting logic lives in the JS layer (chat_ui.html).
//! These Rust-side tests verify that the markdown renderer correctly
//! preserves language fence markers so the JS highlighter can detect them,
//! and that code blocks are rendered with the correct HTML structure.

use openclaw_node_widget_rs::markdown::render_markdown;

// ── Feature: Syntax Highlighting ────────────────────────────────────

// Scenario: Language detected from fence marker
//   Given a code block with ```javascript fence
//   When rendered as markdown
//   Then the output has class="language-javascript" on the code element
#[test]
fn scenario_language_detected_from_fence() {
    let input = "```javascript\nconst x = 1;\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-javascript"),
        "expected language-javascript class, got: {result}"
    );
    assert!(result.contains("<pre><code"));
}

// Scenario: Rust code block preserves language class
//   Given a code block with ```rust fence
//   When rendered as markdown
//   Then the output has class="language-rust"
#[test]
fn scenario_rust_fence_preserved() {
    let input = "```rust\nfn main() {}\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-rust"),
        "got: {result}"
    );
}

// Scenario: Python code block preserves language class
//   Given a code block with ```python fence
//   When rendered as markdown
//   Then the output has class="language-python"
#[test]
fn scenario_python_fence_preserved() {
    let input = "```python\ndef hello():\n    print('hi')\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-python"),
        "got: {result}"
    );
}

// Scenario: TypeScript code block preserves language class
#[test]
fn scenario_typescript_fence_preserved() {
    let input = "```typescript\nconst x: number = 42;\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-typescript"),
        "got: {result}"
    );
}

// Scenario: SQL code block preserves language class
#[test]
fn scenario_sql_fence_preserved() {
    let input = "```sql\nSELECT * FROM users;\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-sql"),
        "got: {result}"
    );
}

// Scenario: Bash code block preserves language class
#[test]
fn scenario_bash_fence_preserved() {
    let input = "```bash\necho 'hello'\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-bash"),
        "got: {result}"
    );
}

// Scenario: JSON code block preserves language class
#[test]
fn scenario_json_fence_preserved() {
    let input = "```json\n{\"key\": \"value\"}\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-json"),
        "got: {result}"
    );
}

// Scenario: HTML code block preserves language class
#[test]
fn scenario_html_fence_preserved() {
    let input = "```html\n<div>hello</div>\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-html"),
        "got: {result}"
    );
}

// Scenario: CSS code block preserves language class
#[test]
fn scenario_css_fence_preserved() {
    let input = "```css\nbody { color: red; }\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("language-css"),
        "got: {result}"
    );
}

// Scenario: Unknown language renders plain
//   Given a code block with an unknown language
//   When rendered as markdown
//   Then the output still has pre/code but with the language class
#[test]
fn scenario_unknown_language_renders_with_class() {
    let input = "```brainfuck\n++++++++++\n```";
    let result = render_markdown(input);
    assert!(
        result.contains("<pre><code"),
        "got: {result}"
    );
    assert!(result.contains("language-brainfuck"));
}

// Scenario: Code block without language
//   Given a code block with no language specified
//   When rendered as markdown
//   Then the output has pre/code without language class
#[test]
fn scenario_no_language_no_class() {
    let input = "```\nsome code\n```";
    let result = render_markdown(input);
    assert!(result.contains("<pre><code>"), "got: {result}");
    assert!(!result.contains("language-"));
}

// Scenario: Code content is HTML-escaped for XSS prevention
//   Given a code block containing HTML tags
//   When rendered as markdown
//   Then the HTML is escaped (not interpreted)
#[test]
fn scenario_code_content_html_escaped() {
    let input = "```javascript\nconst x = '<script>alert(1)</script>';\n```";
    let result = render_markdown(input);
    assert!(
        !result.contains("<script>"),
        "raw script tag should be escaped, got: {result}"
    );
    assert!(result.contains("&lt;script&gt;"));
}

// Scenario: Multiple code blocks in one message
//   Given a message with multiple fenced code blocks
//   When rendered as markdown
//   Then each block has its own pre/code with correct language
#[test]
fn scenario_multiple_code_blocks() {
    let input = "```rust\nfn foo() {}\n```\nsome text\n```python\ndef bar(): pass\n```";
    let result = render_markdown(input);
    assert!(result.contains("language-rust"), "got: {result}");
    assert!(result.contains("language-python"), "got: {result}");
    assert!(result.contains("some text"));
}
