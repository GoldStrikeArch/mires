//! Standalone Rust library compiled to a publishable npm/TypeScript ES module package.
//!
//! Run `cargo mires build --release --npm` to produce a self-contained `.mjs` bundle,
//! TypeScript `.d.ts` declarations, source map, and `package.json` with zero FFI boilerplate.

use std::collections::HashMap;
use std::fmt::Write as _;

use mires::js_export;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderOptions {
    pub heading_ids: Option<bool>,
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOutput {
    pub html: String,
    pub headings: Vec<String>,
    pub word_count: u32,
}

/// Renders a lightweight Markdown subset (`#`/`##` headings, `**bold**`, `` `code` ``,
/// and paragraphs) into HTML with optional deduplicated heading IDs.
#[js_export(name = "renderMarkdown")]
pub fn render_markdown(
    input: &str,
    options: Option<RenderOptions>,
) -> Result<RenderOutput, String> {
    let heading_ids = options
        .as_ref()
        .and_then(|o| o.heading_ids)
        .unwrap_or(false);
    let strict = options.as_ref().and_then(|o| o.strict).unwrap_or(false);

    if strict && input.trim().is_empty() {
        return Err("empty markdown input".to_owned());
    }

    let mut html = String::new();
    let mut headings = Vec::new();
    let mut slug_counts: HashMap<String, u32> = HashMap::new();
    let mut paragraph_lines: Vec<&str> = Vec::new();
    let mut word_count = 0u32;

    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            flush_paragraph(&mut html, &mut paragraph_lines, &mut word_count);
            continue;
        }

        if let Some((level, text)) = parse_heading(line) {
            flush_paragraph(&mut html, &mut paragraph_lines, &mut word_count);
            headings.push(text.to_owned());
            word_count += count_words(text);

            let rendered = render_inline(text);
            if heading_ids {
                let id = next_heading_id(text, &mut slug_counts);
                let _ = writeln!(html, "<h{level} id=\"{id}\">{rendered}</h{level}>");
            } else {
                let _ = writeln!(html, "<h{level}>{rendered}</h{level}>");
            }
        } else {
            paragraph_lines.push(line);
        }
    }

    flush_paragraph(&mut html, &mut paragraph_lines, &mut word_count);

    Ok(RenderOutput {
        html,
        headings,
        word_count,
    })
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    let text = rest.trim();
    if text.is_empty() {
        return None;
    }
    Some((hashes, text))
}

fn flush_paragraph(html: &mut String, lines: &mut Vec<&str>, word_count: &mut u32) {
    if lines.is_empty() {
        return;
    }
    let joined = lines.join(" ");
    lines.clear();
    *word_count += count_words(&joined);
    let rendered = render_inline(&joined);
    let _ = writeln!(html, "<p>{rendered}</p>");
}

fn count_words(text: &str) -> u32 {
    text.split_whitespace().count() as u32
}

fn next_heading_id(text: &str, slug_counts: &mut HashMap<String, u32>) -> String {
    let base = slugify(text);
    let count = slug_counts.entry(base.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base
    } else {
        format!("{base}-{}", *count)
    }
}

fn slugify(text: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !slug.is_empty() && !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "section".to_owned()
    } else {
        slug
    }
}

fn render_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'`' {
            if let Some(rel_end) = text[i + 1..].find('`') {
                let code = &text[i + 1..i + 1 + rel_end];
                out.push_str("<code>");
                push_escaped(&mut out, code);
                out.push_str("</code>");
                i += rel_end + 2;
                continue;
            }
        }

        if bytes[i..].starts_with(b"**") {
            if let Some(rel_end) = text[i + 2..].find("**") {
                if rel_end > 0 {
                    let inner = &text[i + 2..i + 2 + rel_end];
                    out.push_str("<strong>");
                    push_escaped(&mut out, inner);
                    out.push_str("</strong>");
                    i += rel_end + 4;
                    continue;
                }
            }
        }

        let ch = text[i..].chars().next().expect("i is on a char boundary");
        push_escaped_char(&mut out, ch);
        i += ch.len_utf8();
    }

    out
}

fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        push_escaped_char(out, ch);
    }
}

fn push_escaped_char(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        _ => out.push(ch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_headings_paragraphs_bold_and_inline_code() {
        let out = render_markdown("# Hello\n\nSome **bold** and `code`", None).unwrap();
        assert_eq!(
            out.html,
            "<h1>Hello</h1>\n<p>Some <strong>bold</strong> and <code>code</code></p>\n"
        );
        assert_eq!(out.headings, vec!["Hello"]);
        assert_eq!(out.word_count, 5);
    }

    #[test]
    fn deduplicates_heading_ids_with_hashmap() {
        let out = render_markdown(
            "# Hi\n## Sub Section\n# Hi",
            Some(RenderOptions {
                heading_ids: Some(true),
                strict: None,
            }),
        )
        .unwrap();
        assert_eq!(
            out.html,
            "<h1 id=\"hi\">Hi</h1>\n<h2 id=\"sub-section\">Sub Section</h2>\n<h1 id=\"hi-2\">Hi</h1>\n"
        );
        assert_eq!(out.headings, vec!["Hi", "Sub Section", "Hi"]);
        assert_eq!(out.word_count, 4);
    }

    #[test]
    fn strict_mode_rejects_blank_input() {
        let err = render_markdown(
            "   \n\t ",
            Some(RenderOptions {
                heading_ids: None,
                strict: Some(true),
            }),
        )
        .unwrap_err();
        assert_eq!(err, "empty markdown input");

        let ok = render_markdown("   ", None).unwrap();
        assert_eq!(ok.html, "");
        assert!(ok.headings.is_empty());
        assert_eq!(ok.word_count, 0);
    }

    #[test]
    fn escapes_html_in_text_and_code_spans() {
        let out = render_markdown("Use `a < b && c > d` & **<tag>**", None).unwrap();
        assert_eq!(
            out.html,
            "<p>Use <code>a &lt; b &amp;&amp; c &gt; d</code> &amp; <strong>&lt;tag&gt;</strong></p>\n"
        );
    }
}
