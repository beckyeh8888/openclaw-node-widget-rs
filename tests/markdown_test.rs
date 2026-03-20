use openclaw_node_widget_rs::markdown::{escape_html, render_markdown};

// ── Bold ─────────────────────────────────────────────────────────────

#[test]
fn bold_double_asterisks() {
    let result = render_markdown("**hello**");
    assert!(
        result.contains("<strong>hello</strong>"),
        "got: {result}"
    );
}

#[test]
fn bold_double_underscores() {
    let result = render_markdown("__hello__");
    assert!(result.contains("<strong>hello</strong>"));
}

// ── Italic ───────────────────────────────────────────────────────────

#[test]
fn italic_single_asterisk() {
    let result = render_markdown("*hello*");
    assert!(result.contains("<em>hello</em>"), "got: {result}");
}

#[test]
fn italic_single_underscore() {
    let result = render_markdown("_hello_");
    assert!(result.contains("<em>hello</em>"));
}

// ── Inline code ──────────────────────────────────────────────────────

#[test]
fn inline_code() {
    let result = render_markdown("`foo()`");
    assert!(result.contains("<code>foo()</code>"), "got: {result}");
}

// ── Code blocks ──────────────────────────────────────────────────────

#[test]
fn code_block_with_language() {
    let input = "```rust\nfn main() {}\n```";
    let result = render_markdown(input);
    assert!(result.contains("<pre><code class=\"language-rust\">"));
    assert!(result.contains("fn main() {}"));
}

#[test]
fn code_block_without_language() {
    let input = "```\ncode here\n```";
    let result = render_markdown(input);
    assert!(result.contains("<pre><code>"));
    assert!(result.contains("code here"));
}

// ── Links ────────────────────────────────────────────────────────────

#[test]
fn link_renders_anchor() {
    let result = render_markdown("[example](https://example.com)");
    assert!(result.contains("<a href=\"https://example.com\""));
    assert!(result.contains(">example</a>"));
}

// ── Unordered lists ──────────────────────────────────────────────────

#[test]
fn unordered_list_dash() {
    let input = "- one\n- two\n- three";
    let result = render_markdown(input);
    assert!(result.contains("<ul>"), "got: {result}");
    assert!(result.contains("<li>one</li>"));
    assert!(result.contains("<li>two</li>"));
    assert!(result.contains("<li>three</li>"));
    assert!(result.contains("</ul>"));
}

#[test]
fn unordered_list_asterisk() {
    let input = "* alpha\n* beta";
    let result = render_markdown(input);
    assert!(result.contains("<ul>"));
    assert!(result.contains("<li>alpha</li>"));
}

// ── Ordered lists ────────────────────────────────────────────────────

#[test]
fn ordered_list() {
    let input = "1. first\n2. second\n3. third";
    let result = render_markdown(input);
    assert!(result.contains("<ol>"), "got: {result}");
    assert!(result.contains("<li>first</li>"));
    assert!(result.contains("<li>second</li>"));
    assert!(result.contains("</ol>"));
}

// ── Headings ─────────────────────────────────────────────────────────

#[test]
fn h1_renders_as_bold() {
    let result = render_markdown("# Title");
    assert!(result.contains("<strong>Title</strong>"));
}

#[test]
fn h2_renders_as_bold() {
    let result = render_markdown("## Subtitle");
    assert!(result.contains("<strong>Subtitle</strong>"));
}

#[test]
fn h3_renders_as_bold() {
    let result = render_markdown("### Section");
    assert!(result.contains("<strong>Section</strong>"));
}

// ── Nested patterns ──────────────────────────────────────────────────

#[test]
fn bold_inside_sentence() {
    let result = render_markdown("say **hello** world");
    assert!(result.contains("<strong>hello</strong>"));
    assert!(result.contains("say "));
    assert!(result.contains(" world"));
}

// ── Paragraphs ───────────────────────────────────────────────────────

#[test]
fn double_newline_creates_paragraphs() {
    let result = render_markdown("first\n\nsecond");
    assert!(result.contains("</p><p>"));
}

// ── XSS prevention ──────────────────────────────────────────────────

#[test]
fn script_tag_is_escaped() {
    let result = render_markdown("<script>alert(1)</script>");
    assert!(
        !result.contains("<script>"),
        "must not contain raw <script> tag, got: {result}"
    );
    assert!(result.contains("&lt;script&gt;"));
}

#[test]
fn html_entities_are_escaped() {
    let result = render_markdown("<img onerror=\"evil()\">");
    assert!(!result.contains("<img"));
    assert!(result.contains("&lt;img"));
}

#[test]
fn onclick_handler_escaped() {
    let result = render_markdown("<div onclick=\"steal()\">click</div>");
    assert!(!result.contains("<div"));
}

// ── Edge cases ───────────────────────────────────────────────────────

#[test]
fn empty_string() {
    let result = render_markdown("");
    // Should not panic and return something
    let _ = result;
}

#[test]
fn only_backticks_no_closing() {
    let result = render_markdown("```");
    assert!(result.contains("```"));
}

#[test]
fn unmatched_bold_markers() {
    // Should not panic
    let result = render_markdown("**unmatched");
    let _ = result;
}

#[test]
fn unmatched_italic_markers() {
    let result = render_markdown("*unmatched");
    let _ = result;
}

// ── escape_html standalone ───────────────────────────────────────────

#[test]
fn escape_html_angle_brackets() {
    assert_eq!(escape_html("<b>"), "&lt;b&gt;");
}

#[test]
fn escape_html_ampersand() {
    assert_eq!(escape_html("a & b"), "a &amp; b");
}

#[test]
fn escape_html_quotes() {
    assert_eq!(escape_html("\"hello\""), "&quot;hello&quot;");
}
