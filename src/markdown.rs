#![allow(dead_code)]
/// Minimal Markdown-to-HTML renderer (no external dependencies).
///
/// Handles: bold, italic, inline code, code blocks, links,
/// unordered/ordered lists, headings (rendered as bold), paragraphs.
/// All input is HTML-escaped first to prevent XSS.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn render_markdown(text: &str) -> String {
    let escaped = escape_html(text);

    // Split out code blocks first, process inline markdown only outside them
    let mut result = String::new();
    let mut rest = escaped.as_str();

    while let Some(start) = rest.find("```") {
        let before = &rest[..start];
        result.push_str(&render_inline(before));

        let after_tick = &rest[start + 3..];
        // Find the language tag (up to newline) and closing ```
        let (lang, code_start) = match after_tick.find('\n') {
            Some(nl) => {
                let lang_candidate = &after_tick[..nl];
                // Only treat as lang if it's a single word with no spaces
                if !lang_candidate.is_empty()
                    && lang_candidate.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                {
                    (Some(lang_candidate), nl + 1)
                } else {
                    (None, 0)
                }
            }
            None => (None, 0),
        };

        let code_body = &after_tick[code_start..];
        if let Some(end) = code_body.find("```") {
            let code = &code_body[..end];
            match lang {
                Some(l) => {
                    result.push_str(&format!(
                        "<pre><code class=\"language-{l}\">{code}</code></pre>"
                    ));
                }
                None => {
                    result.push_str(&format!("<pre><code>{code}</code></pre>"));
                }
            }
            rest = &code_body[end + 3..];
        } else {
            // No closing ```, treat literally
            result.push_str("```");
            rest = after_tick;
        }
    }
    result.push_str(&render_inline(rest));
    result
}

fn render_inline(text: &str) -> String {
    let mut s = text.to_string();

    // Headings (### / ## / #) → bold (before inline formatting)
    s = render_headings(&s);

    // Lists (before italic to avoid `* item` being treated as italic)
    s = render_lists(&s);

    // Inline code (must come before bold/italic to avoid conflicts)
    s = replace_pattern(&s, "`", "`", "code");

    // Bold **text** and __text__
    s = replace_delimited(&s, "**", "**", "strong");
    s = replace_delimited(&s, "__", "__", "strong");

    // Italic *text* and _text_
    s = replace_delimited(&s, "*", "*", "em");
    s = replace_delimited(&s, "_", "_", "em");

    // Links [text](url)
    s = render_links(&s);

    // Paragraphs: double newline
    s = s.replace("\n\n", "</p><p>");

    // Single newlines → <br>
    s = s.replace('\n', "<br>");

    s
}

fn replace_pattern(text: &str, open: &str, close: &str, tag: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        result.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        if let Some(end) = after.find(close) {
            let inner = &after[..end];
            if !inner.is_empty() {
                result.push_str(&format!("<{tag}>{inner}</{tag}>"));
                rest = &after[end + close.len()..];
            } else {
                result.push_str(open);
                rest = after;
            }
        } else {
            result.push_str(open);
            rest = after;
        }
    }
    result.push_str(rest);
    result
}

fn replace_delimited(text: &str, open: &str, close: &str, tag: &str) -> String {
    replace_pattern(text, open, close, tag)
}

fn render_links(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(bracket_start) = rest.find('[') {
        result.push_str(&rest[..bracket_start]);
        let after_bracket = &rest[bracket_start + 1..];
        if let Some(bracket_end) = after_bracket.find("](") {
            let link_text = &after_bracket[..bracket_end];
            let after_paren = &after_bracket[bracket_end + 2..];
            if let Some(paren_end) = after_paren.find(')') {
                let url = &after_paren[..paren_end];
                result.push_str(&format!(
                    "<a href=\"{url}\" target=\"_blank\" rel=\"noopener\">{link_text}</a>"
                ));
                rest = &after_paren[paren_end + 1..];
            } else {
                result.push('[');
                rest = after_bracket;
            }
        } else {
            result.push('[');
            rest = after_bracket;
        }
    }
    result.push_str(rest);
    result
}

fn render_headings(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        if let Some(content) = line.strip_prefix("### ") {
            lines.push(format!("<strong>{content}</strong>"));
        } else if let Some(content) = line.strip_prefix("## ") {
            lines.push(format!("<strong>{content}</strong>"));
        } else if let Some(content) = line.strip_prefix("# ") {
            lines.push(format!("<strong>{content}</strong>"));
        } else {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

fn render_lists(text: &str) -> String {
    let mut result = String::new();
    let mut in_ul = false;
    let mut in_ol = false;

    for line in text.split('\n') {
        let trimmed = line.trim_start();
        let is_ul = trimmed.starts_with("- ") || trimmed.starts_with("* ");
        let is_ol = is_ordered_list_item(trimmed);

        if is_ul {
            if !in_ul {
                if in_ol {
                    result.push_str("</ol>");
                    in_ol = false;
                }
                result.push_str("<ul>");
                in_ul = true;
            }
            let content = trimmed.strip_prefix("- ").unwrap_or(&trimmed[2..]);
            result.push_str(&format!("<li>{content}</li>"));
        } else if is_ol {
            if !in_ol {
                if in_ul {
                    result.push_str("</ul>");
                    in_ul = false;
                }
                result.push_str("<ol>");
                in_ol = true;
            }
            let content = trimmed.split_once(". ").map(|x| x.1).unwrap_or("");
            result.push_str(&format!("<li>{content}</li>"));
        } else {
            if in_ul {
                result.push_str("</ul>");
                in_ul = false;
            }
            if in_ol {
                result.push_str("</ol>");
                in_ol = false;
            }
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
        }
    }
    if in_ul {
        result.push_str("</ul>");
    }
    if in_ol {
        result.push_str("</ol>");
    }
    result
}

fn is_ordered_list_item(line: &str) -> bool {
    let mut chars = line.chars();
    let first = chars.next();
    match first {
        Some(c) if c.is_ascii_digit() => {
            // consume remaining digits
            let rest: String = chars.collect();
            rest.starts_with(". ") || rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && rest.contains(". ")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Bold ─────────────────────────────────────────────────────

    #[test]
    fn given_bold_markdown_when_rendered_then_output_contains_strong() {
        let result = render_markdown("**hello**");
        assert!(
            result.contains("<strong>hello</strong>"),
            "expected <strong>hello</strong>, got: {result}"
        );
    }

    #[test]
    fn given_underscore_bold_when_rendered_then_output_contains_strong() {
        let result = render_markdown("__world__");
        assert!(result.contains("<strong>world</strong>"));
    }

    // ── Italic ───────────────────────────────────────────────────

    #[test]
    fn given_italic_markdown_when_rendered_then_output_contains_em() {
        let result = render_markdown("*hello*");
        assert!(result.contains("<em>hello</em>"));
    }

    #[test]
    fn given_underscore_italic_when_rendered_then_output_contains_em() {
        let result = render_markdown("_world_");
        assert!(result.contains("<em>world</em>"));
    }

    // ── Inline code ──────────────────────────────────────────────

    #[test]
    fn given_inline_code_when_rendered_then_output_contains_code() {
        let result = render_markdown("`foo()`");
        assert!(
            result.contains("<code>foo()</code>"),
            "got: {result}"
        );
    }

    // ── Code blocks ──────────────────────────────────────────────

    #[test]
    fn given_code_block_when_rendered_then_output_contains_pre_code() {
        let input = "```rust\nfn main() {}\n```";
        let result = render_markdown(input);
        assert!(result.contains("<pre><code"), "got: {result}");
        assert!(result.contains("fn main() {}"));
    }

    #[test]
    fn given_code_block_without_lang_when_rendered_then_pre_code_present() {
        let input = "```\nsome code\n```";
        let result = render_markdown(input);
        assert!(result.contains("<pre><code>"));
        assert!(result.contains("some code"));
    }

    // ── Links ────────────────────────────────────────────────────

    #[test]
    fn given_link_when_rendered_then_output_contains_anchor() {
        let result = render_markdown("[click](https://example.com)");
        assert!(result.contains("<a href=\"https://example.com\""));
        assert!(result.contains(">click</a>"));
    }

    // ── Unordered lists ──────────────────────────────────────────

    #[test]
    fn given_unordered_list_when_rendered_then_output_contains_ul_li() {
        let input = "- item one\n- item two";
        let result = render_markdown(input);
        assert!(result.contains("<ul>"), "got: {result}");
        assert!(result.contains("<li>item one</li>"));
        assert!(result.contains("<li>item two</li>"));
    }

    // ── Ordered lists ────────────────────────────────────────────

    #[test]
    fn given_ordered_list_when_rendered_then_output_contains_ol_li() {
        let input = "1. first\n2. second";
        let result = render_markdown(input);
        assert!(result.contains("<ol>"), "got: {result}");
        assert!(result.contains("<li>first</li>"));
        assert!(result.contains("<li>second</li>"));
    }

    // ── Headings ─────────────────────────────────────────────────

    #[test]
    fn given_heading_when_rendered_then_output_contains_bold() {
        let result = render_markdown("# Title");
        assert!(result.contains("<strong>Title</strong>"));
    }

    #[test]
    fn given_h2_when_rendered_then_output_contains_bold() {
        let result = render_markdown("## Subtitle");
        assert!(result.contains("<strong>Subtitle</strong>"));
    }

    #[test]
    fn given_h3_when_rendered_then_output_contains_bold() {
        let result = render_markdown("### Section");
        assert!(result.contains("<strong>Section</strong>"));
    }

    // ── Paragraphs ───────────────────────────────────────────────

    #[test]
    fn given_double_newline_when_rendered_then_paragraphs_created() {
        let result = render_markdown("first\n\nsecond");
        assert!(result.contains("</p><p>"));
    }

    // ── XSS prevention ──────────────────────────────────────────

    #[test]
    fn given_script_tag_when_rendered_then_escaped() {
        let result = render_markdown("<script>alert(1)</script>");
        assert!(
            !result.contains("<script>"),
            "must not contain raw <script> tag, got: {result}"
        );
        assert!(result.contains("&lt;script&gt;"));
    }

    #[test]
    fn given_html_entities_when_rendered_then_escaped() {
        let result = render_markdown("<div onclick=\"evil()\">hi</div>");
        assert!(!result.contains("<div"));
        assert!(result.contains("&lt;div"));
    }

    // ── Edge cases ───────────────────────────────────────────────

    #[test]
    fn given_empty_string_when_rendered_then_no_panic() {
        let result = render_markdown("");
        // Should not panic; empty input produces empty output
        let _ = result;
    }

    #[test]
    fn given_only_backticks_when_rendered_then_no_panic() {
        let result = render_markdown("```");
        assert!(result.contains("```"));
    }

    #[test]
    fn given_unmatched_bold_markers_when_rendered_then_no_crash() {
        let result = render_markdown("**unmatched");
        assert!(result.contains("**unmatched") || result.contains("unmatched"));
    }

    // ── Nested patterns ──────────────────────────────────────────

    #[test]
    fn given_bold_inside_text_when_rendered_then_strong_tag_present() {
        let result = render_markdown("say **hello** world");
        assert!(result.contains("<strong>hello</strong>"));
        assert!(result.contains("say "));
        assert!(result.contains(" world"));
    }

    // ── escape_html ──────────────────────────────────────────────

    #[test]
    fn given_angle_brackets_then_escaped() {
        assert_eq!(escape_html("<b>"), "&lt;b&gt;");
    }

    #[test]
    fn given_ampersand_then_escaped() {
        assert_eq!(escape_html("a&b"), "a&amp;b");
    }
}
