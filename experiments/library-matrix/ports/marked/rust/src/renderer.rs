//! Parser, Renderer, and TextRenderer mirroring `src/Parser.ts`, `src/Renderer.ts`, and
//! `src/TextRenderer.ts` of marked 18.1.1.

use alloc::string::String;

use crate::helpers::{clean_url, push_escaped_html, push_u64};
use crate::tokens::{Align, BlockToken, InlineSlot, InlineToken};
use crate::unicode::is_js_space;

/// Renders block tokens into an HTML string (`Parser.parse` + `Renderer`).
pub fn render_blocks(tokens: &[BlockToken], slots: &[InlineSlot], out: &mut String) {
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            BlockToken::Space { .. } | BlockToken::Def { .. } => {}
            BlockToken::Hr { .. } => out.push_str("<hr>\n"),
            BlockToken::Heading {
                depth, inline_id, ..
            } => {
                out.push_str("<h");
                push_u64(out, *depth as u64);
                out.push('>');
                render_inline(&slots[*inline_id].tokens, out);
                out.push_str("</h");
                push_u64(out, *depth as u64);
                out.push_str(">\n");
            }
            BlockToken::Code { lang, text, .. } => {
                let lang_str = match lang {
                    Some(l) => first_non_space_run(l),
                    None => "",
                };
                if lang_str.is_empty() {
                    out.push_str("<pre><code>");
                } else {
                    out.push_str("<pre><code class=\"language-");
                    push_escaped_html(out, lang_str, false);
                    out.push_str("\">");
                }
                if !text.is_empty() {
                    let trimmed = text.strip_suffix('\n').unwrap_or(text.as_str());
                    push_escaped_html(out, trimmed, true);
                    out.push('\n');
                }
                out.push_str("</code></pre>\n");
            }
            BlockToken::Blockquote { tokens: inner, .. } => {
                out.push_str("<blockquote>\n");
                render_blocks(inner, slots, out);
                out.push_str("</blockquote>\n");
            }
            BlockToken::Html { text, .. } => {
                out.push_str(text);
            }
            BlockToken::List {
                ordered,
                start,
                items,
                ..
            } => {
                if *ordered {
                    out.push_str("<ol");
                    if *start != 1 {
                        out.push_str(" start=\"");
                        push_u64(out, *start);
                        out.push('"');
                    }
                    out.push_str(">\n");
                } else {
                    out.push_str("<ul>\n");
                }
                let mut j = 0;
                while j < items.len() {
                    out.push_str("<li>");
                    render_blocks(&items[j].tokens, slots, out);
                    out.push_str("</li>\n");
                    j += 1;
                }
                if *ordered {
                    out.push_str("</ol>\n");
                } else {
                    out.push_str("</ul>\n");
                }
            }
            BlockToken::Checkbox { checked, .. } => {
                render_checkbox(*checked, out);
            }
            BlockToken::Paragraph { inline_id, .. } => {
                out.push_str("<p>");
                render_inline(&slots[*inline_id].tokens, out);
                out.push_str("</p>\n");
            }
            BlockToken::Text { inline_id, .. } => {
                render_inline(&slots[*inline_id].tokens, out);
            }
            BlockToken::Table { header, rows, .. } => {
                out.push_str("<table>\n<thead>\n<tr>\n");
                let mut j = 0;
                while j < header.len() {
                    render_table_cell(&header[j], slots, out);
                    j += 1;
                }
                out.push_str("</tr>\n</thead>\n");
                if !rows.is_empty() {
                    out.push_str("<tbody>");
                    let mut r = 0;
                    while r < rows.len() {
                        out.push_str("<tr>\n");
                        let row = &rows[r];
                        let mut c = 0;
                        while c < row.len() {
                            render_table_cell(&row[c], slots, out);
                            c += 1;
                        }
                        out.push_str("</tr>\n");
                        r += 1;
                    }
                    out.push_str("</tbody>");
                }
                out.push_str("</table>\n");
            }
        }
        i += 1;
    }
}

fn render_table_cell(cell: &crate::tokens::TableCell, slots: &[InlineSlot], out: &mut String) {
    let tag = if cell.header { "th" } else { "td" };
    out.push('<');
    out.push_str(tag);
    match cell.align {
        Align::None => {}
        Align::Left => out.push_str(" align=\"left\""),
        Align::Center => out.push_str(" align=\"center\""),
        Align::Right => out.push_str(" align=\"right\""),
    }
    out.push('>');
    render_inline(&slots[cell.inline_id].tokens, out);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

#[inline]
fn render_checkbox(checked: bool, out: &mut String) {
    out.push_str("<input ");
    if checked {
        out.push_str("checked=\"\" ");
    }
    out.push_str("disabled=\"\" type=\"checkbox\"> ");
}

/// Renders inline tokens into `out` (`Parser.parseInline` + `Renderer`).
pub fn render_inline(tokens: &[InlineToken], out: &mut String) {
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            InlineToken::Escape { text } => {
                push_escaped_html(out, text, false);
            }
            InlineToken::Html { text } => {
                out.push_str(text);
            }
            InlineToken::Checkbox { checked, .. } => {
                render_checkbox(*checked, out);
            }
            InlineToken::Strong { tokens: inner, .. } => {
                out.push_str("<strong>");
                render_inline(inner, out);
                out.push_str("</strong>");
            }
            InlineToken::Em { tokens: inner, .. } => {
                out.push_str("<em>");
                render_inline(inner, out);
                out.push_str("</em>");
            }
            InlineToken::Codespan { text } => {
                out.push_str("<code>");
                push_escaped_html(out, text, true);
                out.push_str("</code>");
            }
            InlineToken::Br => {
                out.push_str("<br>");
            }
            InlineToken::Del { tokens: inner, .. } => {
                out.push_str("<del>");
                render_inline(inner, out);
                out.push_str("</del>");
            }
            InlineToken::Link {
                href,
                title,
                text,
                autolink,
                tokens: inner,
            } => {
                let clean = clean_url(href);
                out.push_str("<a href=\"");
                push_escaped_html(out, &clean, *autolink);
                out.push('"');
                if let Some(t) = title {
                    if !t.is_empty() {
                        out.push_str(" title=\"");
                        push_escaped_html(out, t, false);
                        out.push('"');
                    }
                }
                out.push('>');
                if *autolink {
                    push_escaped_html(out, text, true);
                } else {
                    render_inline(inner, out);
                }
                out.push_str("</a>");
            }
            InlineToken::Image {
                href,
                title,
                tokens: inner,
                ..
            } => {
                let mut alt = String::new();
                render_text_inline(inner, &mut alt);
                let clean = clean_url(href);
                out.push_str("<img src=\"");
                push_escaped_html(out, &clean, false);
                out.push_str("\" alt=\"");
                push_escaped_html(out, &alt, false);
                out.push('"');
                if let Some(t) = title {
                    if !t.is_empty() {
                        out.push_str(" title=\"");
                        push_escaped_html(out, t, false);
                        out.push('"');
                    }
                }
                out.push('>');
            }
            InlineToken::Text { text, escaped, .. } => {
                if *escaped {
                    out.push_str(text);
                } else {
                    push_escaped_html(out, text, false);
                }
            }
        }
        i += 1;
    }
}

/// Exact port of `TextRenderer` (`src/TextRenderer.ts`) used by `Renderer.image` for `alt` text.
fn render_text_inline(tokens: &[InlineToken], out: &mut String) {
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            InlineToken::Strong { text, .. }
            | InlineToken::Em { text, .. }
            | InlineToken::Codespan { text }
            | InlineToken::Del { text, .. }
            | InlineToken::Html { text }
            | InlineToken::Text { text, .. }
            | InlineToken::Escape { text }
            | InlineToken::Link { text, .. }
            | InlineToken::Image { text, .. } => out.push_str(text),
            InlineToken::Br => {}
            InlineToken::Checkbox { raw, .. } => out.push_str(raw),
        }
        i += 1;
    }
}

/// Matches `other.notSpaceStart` (`^\S*`).
fn first_non_space_run(s: &str) -> &str {
    let mut end = 0;
    for ch in s.chars() {
        if is_js_space(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    &s[..end]
}
