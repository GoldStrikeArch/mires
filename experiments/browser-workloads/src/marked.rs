//! A streaming Markdown workload inspired by Marked.

use alloc::string::String;

/// Renders a practical Markdown subset to deterministic HTML.
#[must_use]
pub fn render(markdown: &str) -> String {
    let mut html = String::new();
    let mut paragraph = String::new();
    let mut in_list = false;
    let mut in_code = false;
    let bytes = markdown.as_bytes();
    let mut line_start = 0;
    while line_start <= bytes.len() {
        let line_end = find_byte(bytes, line_start, b'\n').unwrap_or(bytes.len());
        let line = &markdown[line_start..line_end];
        if line.starts_with("```") {
            flush_paragraph(&mut html, &mut paragraph);
            close_list(&mut html, &mut in_list);
            if in_code {
                html.push_str("</code></pre>\n");
            } else {
                html.push_str("<pre><code>");
            }
            in_code = !in_code;
            if line_end == bytes.len() { break } else { line_start = line_end + 1; continue }
        }
        if in_code {
            escape_into(&mut html, line);
            html.push('\n');
            if line_end == bytes.len() { break } else { line_start = line_end + 1; continue }
        }
        if let Some((level, title)) = heading(line) {
            flush_paragraph(&mut html, &mut paragraph);
            close_list(&mut html, &mut in_list);
            html.push('<');
            html.push('h');
            html.push(char::from(b'0' + level));
            html.push('>');
            inline_into(&mut html, title);
            html.push_str("</h");
            html.push(char::from(b'0' + level));
            html.push_str(">\n");
        } else if let Some(item) = line.strip_prefix("- ") {
            flush_paragraph(&mut html, &mut paragraph);
            if !in_list {
                html.push_str("<ul>\n");
                in_list = true;
            }
            html.push_str("<li>");
            inline_into(&mut html, item);
            html.push_str("</li>\n");
        } else if let Some(quote) = line.strip_prefix("> ") {
            flush_paragraph(&mut html, &mut paragraph);
            close_list(&mut html, &mut in_list);
            html.push_str("<blockquote>");
            inline_into(&mut html, quote);
            html.push_str("</blockquote>\n");
        } else if trim_ascii(line).is_empty() {
            flush_paragraph(&mut html, &mut paragraph);
            close_list(&mut html, &mut in_list);
        } else {
            close_list(&mut html, &mut in_list);
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(trim_ascii(line));
        }
        if line_end == bytes.len() {
            break;
        }
        line_start = line_end + 1;
    }
    flush_paragraph(&mut html, &mut paragraph);
    close_list(&mut html, &mut in_list);
    if in_code {
        html.push_str("</code></pre>\n");
    }
    html
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let bytes = line.as_bytes();
    let mut level = 0;
    while level < bytes.len() && bytes[level] == b'#' {
        level += 1;
    }
    if level > 0 && level <= 6 && level < bytes.len() && bytes[level] == b' ' {
        Some((level as u8, &line[level + 1..]))
    } else {
        None
    }
}

fn flush_paragraph(html: &mut String, paragraph: &mut String) {
    if paragraph.is_empty() {
        return;
    }
    html.push_str("<p>");
    inline_into(html, paragraph);
    html.push_str("</p>\n");
    paragraph.clear();
}

fn close_list(html: &mut String, in_list: &mut bool) {
    if *in_list {
        html.push_str("</ul>\n");
        *in_list = false;
    }
}

fn inline_into(output: &mut String, source: &str) {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if source[index..].starts_with("**") {
            if let Some(end) = source[index + 2..].find("**") {
                output.push_str("<strong>");
                escape_into(output, &source[index + 2..index + 2 + end]);
                output.push_str("</strong>");
                index += end + 4;
                continue;
            }
        } else if bytes[index] == b'*' {
            if let Some(end) = find_byte(bytes, index + 1, b'*').map(|end| end - index - 1) {
                output.push_str("<em>");
                escape_into(output, &source[index + 1..index + 1 + end]);
                output.push_str("</em>");
                index += end + 2;
                continue;
            }
        } else if bytes[index] == b'`' {
            if let Some(end) = find_byte(bytes, index + 1, b'`').map(|end| end - index - 1) {
                output.push_str("<code>");
                escape_into(output, &source[index + 1..index + 1 + end]);
                output.push_str("</code>");
                index += end + 2;
                continue;
            }
        } else if bytes[index] == b'[' {
            if let Some(label_end) = source[index + 1..].find("](") {
                let url_start = index + label_end + 3;
                if let Some(url_end) = find_byte(bytes, url_start, b')').map(|end| end - url_start) {
                    output.push_str("<a href=\"");
                    escape_attribute_into(output, &source[url_start..url_start + url_end]);
                    output.push_str("\">");
                    escape_into(output, &source[index + 1..index + 1 + label_end]);
                    output.push_str("</a>");
                    index = url_start + url_end + 1;
                    continue;
                }
            }
        }
        let width = utf8_width(bytes[index]).min(bytes.len() - index);
        escape_slice(output, &source[index..index + width]);
        index += width;
    }
}

fn escape_into(output: &mut String, source: &str) {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let width = utf8_width(bytes[index]).min(bytes.len() - index);
        escape_slice(output, &source[index..index + width]);
        index += width;
    }
}

fn escape_attribute_into(output: &mut String, source: &str) {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let width = utf8_width(bytes[index]).min(bytes.len() - index);
        if bytes[index] == b'"' {
            output.push_str("&quot;");
        } else {
            escape_slice(output, &source[index..index + width]);
        }
        index += width;
    }
}

fn escape_slice(output: &mut String, character: &str) {
    match character.as_bytes() {
        b"&" => output.push_str("&amp;"),
        b"<" => output.push_str("&lt;"),
        b">" => output.push_str("&gt;"),
        _ => output.push_str(character),
    }
}

fn utf8_width(first: u8) -> usize {
    match first {
        0..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

fn find_byte(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == needle {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn trim_ascii(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut start = 0;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &value[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_blocks_and_inline_markup() {
        assert_eq!(
            render("# Hi\n\n- **one**\n- `two`"),
            "<h1>Hi</h1>\n<ul>\n<li><strong>one</strong></li>\n<li><code>two</code></li>\n</ul>\n"
        );
    }
}
