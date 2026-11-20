//! Exact ports of `src/helpers.ts` from marked 18.1.1.

use alloc::string::String;
use alloc::vec::Vec;

use crate::unicode::{is_js_space, is_punct, trim_js};

#[inline]
fn is_ascii_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Checks whether `bytes[amp_pos..]` starts with an HTML entity reference matching
/// `&(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);`.
pub fn is_entity_at(bytes: &[u8], amp_pos: usize) -> bool {
    let n = bytes.len();
    if amp_pos + 2 >= n || bytes[amp_pos] != b'&' {
        return false;
    }
    let i = amp_pos + 1;
    if bytes[i] == b'#' {
        if i + 1 >= n {
            return false;
        }
        if bytes[i + 1] == b'x' || bytes[i + 1] == b'X' {
            let mut j = i + 2;
            let mut digits = 0;
            while j < n && digits < 6 && bytes[j].is_ascii_hexdigit() {
                j += 1;
                digits += 1;
            }
            digits >= 1 && j < n && bytes[j] == b';'
        } else {
            let mut j = i + 1;
            let mut digits = 0;
            while j < n && digits < 7 && bytes[j].is_ascii_digit() {
                j += 1;
                digits += 1;
            }
            digits >= 1 && j < n && bytes[j] == b';'
        }
    } else {
        let mut j = i;
        while j < n && is_ascii_word(bytes[j]) {
            j += 1;
        }
        j > i && j < n && bytes[j] == b';'
    }
}

/// Appends HTML-escaped `s` to `out` (`escapeHtmlEntities` in `helpers.ts`).
pub fn push_escaped_html(out: &mut String, s: &str, encode: bool) {
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut last = 0usize;
    let mut i = 0usize;
    while i < n {
        let repl = match bytes[i] {
            b'&' => {
                if encode || !is_entity_at(bytes, i) {
                    "&amp;"
                } else {
                    i += 1;
                    continue;
                }
            }
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            b'\'' => "&#39;",
            _ => {
                i += 1;
                continue;
            }
        };
        if i > last {
            out.push_str(&s[last..i]);
        }
        out.push_str(repl);
        i += 1;
        last = i;
    }
    if last < n {
        out.push_str(&s[last..]);
    }
}

/// Attempts to parse a numeric character reference at `bytes[pos..]` matching
/// `/&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));/`. Returns `(end_byte_idx, code_point)`.
fn parse_numeric_char_ref(bytes: &[u8], pos: usize) -> Option<(usize, u32)> {
    let n = bytes.len();
    if pos + 3 >= n || bytes[pos] != b'&' || bytes[pos + 1] != b'#' {
        return None;
    }
    let i = pos + 2;
    if bytes[i] == b'x' || bytes[i] == b'X' {
        let mut j = i + 1;
        let mut digits = 0usize;
        let mut val = 0u32;
        while j < n && digits < 6 && bytes[j].is_ascii_hexdigit() {
            let b = bytes[j];
            let d = if b <= b'9' {
                (b - b'0') as u32
            } else if b <= b'F' {
                (b - b'A' + 10) as u32
            } else {
                (b - b'a' + 10) as u32
            };
            val = (val << 4) | d;
            j += 1;
            digits += 1;
        }
        if digits >= 1 && j < n && bytes[j] == b';' {
            return Some((j + 1, val));
        }
    } else {
        let mut j = i;
        let mut digits = 0usize;
        let mut val = 0u32;
        while j < n && digits < 7 && bytes[j].is_ascii_digit() {
            val = val * 10 + (bytes[j] - b'0') as u32;
            j += 1;
            digits += 1;
        }
        if digits >= 1 && j < n && bytes[j] == b';' {
            return Some((j + 1, val));
        }
    }
    None
}

/// Appends `decodeNumericCharacterReferences(text)` directly into `out` without extra allocation.
pub fn decode_numeric_character_references_into(text: &str, out: &mut String) {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut has_ncr = false;
    let mut scan = 0;
    while scan + 3 < n {
        if bytes[scan] == b'&' && bytes[scan + 1] == b'#' && parse_numeric_char_ref(bytes, scan).is_some() {
            has_ncr = true;
            break;
        }
        scan += 1;
    }
    if !has_ncr {
        out.push_str(text);
        return;
    }

    let mut last = 0usize;
    let mut i = 0usize;
    while i < n {
        if bytes[i] == b'&' {
            if let Some((end, code)) = parse_numeric_char_ref(bytes, i) {
                if i > last {
                    out.push_str(&text[last..i]);
                }
                if code == 0 || code > 0x10FFFF || (0xD800..=0xDFFF).contains(&code) {
                    out.push('\u{FFFD}');
                } else if code == 38 {
                    out.push_str("&amp;");
                } else if let Some(ch) = char::from_u32(code) {
                    out.push(ch);
                } else {
                    out.push('\u{FFFD}');
                }
                i = end;
                last = end;
                continue;
            }
            if !is_entity_at(bytes, i) {
                if i > last {
                    out.push_str(&text[last..i]);
                }
                out.push_str("&amp;");
                i += 1;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    if last < n {
        out.push_str(&text[last..]);
    }
}

/// Exact port of `cleanUrl` (`encodeURI(href).replace(/%25/g, '%')`).
/// For well-formed UTF-8 strings, keeps ASCII bytes in `encodeURI`'s unescaped set plus `%`
/// verbatim and percent-encodes all other UTF-8 bytes as uppercase `%XX`.
pub fn clean_url(href: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let bytes = href.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut last = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        let keep = b < 0x80
            && (b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b'-'
                        | b'.'
                        | b'/'
                        | b':'
                        | b';'
                        | b'='
                        | b'?'
                        | b'@'
                        | b'_'
                        | b'~'
                ));
        if !keep {
            if i > last {
                out.push_str(&href[last..i]);
            }
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0F) as usize] as char);
            last = i + 1;
        }
        i += 1;
    }
    if last < bytes.len() {
        out.push_str(&href[last..]);
    }
    out
}

/// Exact port of `splitCells(tableRow, count)` in `helpers.ts`.
pub fn split_cells(table_row: &str, count: Option<usize>) -> Vec<String> {
    let bytes = table_row.as_bytes();
    let n = bytes.len();
    let mut spaced = String::with_capacity(n + 8);
    let mut last = 0usize;
    let mut i = 0usize;
    while i < n {
        if bytes[i] == b'|' {
            let mut escaped = false;
            let mut curr = i;
            while curr > 0 && bytes[curr - 1] == b'\\' {
                escaped = !escaped;
                curr -= 1;
            }
            if !escaped {
                if i > last {
                    spaced.push_str(&table_row[last..i]);
                }
                spaced.push_str(" |");
                last = i + 1;
            }
        }
        i += 1;
    }
    if last < n {
        spaced.push_str(&table_row[last..]);
    }

    let mut cells: Vec<&str> = spaced.split(" |").collect();
    if !cells.is_empty() && trim_js(cells[0]).is_empty() {
        cells.remove(0);
    }
    if let Some(&last_cell) = cells.last() {
        if trim_js(last_cell).is_empty() {
            cells.pop();
        }
    }

    let mut out: Vec<String> = Vec::with_capacity(count.unwrap_or(cells.len()));
    let target = match count {
        Some(c) if c > 0 => c,
        _ => cells.len(),
    };
    let take = if cells.len() > target { target } else { cells.len() };
    let mut k = 0;
    while k < take {
        out.push(replace_slash_pipe(trim_js(cells[k])));
        k += 1;
    }
    while out.len() < target {
        out.push(String::new());
    }
    out
}

fn replace_slash_pipe(s: &str) -> String {
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n);
    let mut i = 0;
    let mut last = 0;
    while i + 1 < n {
        if bytes[i] == b'\\' && bytes[i + 1] == b'|' {
            if i > last {
                out.push_str(&s[last..i]);
            }
            out.push('|');
            i += 2;
            last = i;
        } else {
            i += 1;
        }
    }
    if last < n {
        out.push_str(&s[last..]);
    }
    out
}

/// Remove trailing ASCII bytes equal to `ch` (`rtrim(str, c)` in `helpers.ts`).
#[inline]
pub fn rtrim_byte(s: &str, ch: u8) -> &str {
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == ch {
        end -= 1;
    }
    &s[..end]
}

/// Checks `/^[ \t]*$/`.
#[inline]
pub fn is_blank_line(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b' ' && b != b'\t' {
            return false;
        }
        i += 1;
    }
    true
}

/// Exact port of `trimTrailingBlankLines` in `helpers.ts`.
pub fn trim_trailing_blank_lines(s: &str) -> &str {
    let bytes = s.as_bytes();
    // Collect line start/end slices without allocating strings
    let mut line_starts: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line_starts.push((start, i));
            start = i + 1;
        }
        i += 1;
    }
    line_starts.push((start, bytes.len()));

    let mut end_idx = line_starts.len() as isize - 1;
    while end_idx >= 0 {
        let (a, b) = line_starts[end_idx as usize];
        if is_blank_line(&s[a..b]) {
            end_idx -= 1;
        } else {
            break;
        }
    }
    if (line_starts.len() as isize) - end_idx <= 2 {
        return s;
    }
    if end_idx < 0 {
        return "";
    }
    let (_, slice_end) = line_starts[end_idx as usize];
    &s[..slice_end]
}

/// Exact port of `normalizeLabel(label)`: `label.trim().toLowerCase().toUpperCase().toLowerCase()`.
pub fn normalize_label(label: &str) -> String {
    let trimmed = trim_js(label);
    // WORKAROUND(MIRES-018): `str::is_ascii()` uses `core` SWAR unaligned `u32` reads, which throw
    // `byte offset N cannot form a u32 scalar view` under `js-str=bytes`; scan bytes directly.
    let bytes = trimmed.as_bytes();
    let mut all_ascii = true;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] >= 0x80 {
            all_ascii = false;
            break;
        }
        i += 1;
    }
    if all_ascii {
        let mut out = String::with_capacity(bytes.len());
        let mut j = 0;
        while j < bytes.len() {
            out.push(bytes[j].to_ascii_lowercase() as char);
            j += 1;
        }
        return out;
    }
    trimmed.to_lowercase().to_uppercase().to_lowercase()
}

/// Collapses runs of JS whitespace (`/\s+/g`) to a single `' '`.
pub fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for ch in s.chars() {
        if is_js_space(ch) {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            in_space = false;
            out.push(ch);
        }
    }
    out
}

/// Exact port of `findClosingBracket(str, "()")` in `helpers.ts`.
pub fn find_closing_paren(s: &str) -> isize {
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut has_paren = false;
    let mut k = 0;
    while k < n {
        if bytes[k] == b'(' || bytes[k] == b')' {
            has_paren = true;
            break;
        }
        k += 1;
    }
    if !has_paren {
        return -1;
    }

    let mut level = 0isize;
    let mut i = 0usize;
    while i < n {
        let b = bytes[i];
        if b == b'\\' {
            // Skip next character (ASCII or UTF-8 scalar)
            i += 1;
            if i < n {
                if bytes[i] < 0x80 {
                    i += 1;
                } else if let Some(ch) = s[i..].chars().next() {
                    i += ch.len_utf8();
                }
            }
            continue;
        }
        if b == b'(' {
            level += 1;
        } else if b == b')' {
            level -= 1;
            if level < 0 {
                return i as isize;
            }
        }
        i += 1;
    }
    if level > 0 {
        -2
    } else {
        -1
    }
}

/// Exact port of `expandTabs(line, indent)` in `helpers.ts`.
/// Note JS `for (const char of line)` iterates Unicode scalar values (code points).
pub fn expand_tabs(line: &str, indent: usize) -> String {
    if !line.as_bytes().contains(&b'\t') {
        return String::from(line);
    }
    let mut col = indent;
    let mut out = String::with_capacity(line.len() + 8);
    for ch in line.chars() {
        if ch == '\t' {
            let added = 4 - (col % 4);
            let mut k = 0;
            while k < added {
                out.push(' ');
                k += 1;
            }
            col += added;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// Replaces `/\\([\p{P}\p{S}])/gu` with `$1` (`rules.inline.anyPunctuation`).
pub fn unescape_any_punctuation(s: &str) -> String {
    if !s.as_bytes().contains(&b'\\') {
        return String::from(s);
    }
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n);
    let mut last = 0usize;
    let mut i = 0usize;
    while i < n {
        if bytes[i] == b'\\' && i + 1 < n {
            if let Some(next_ch) = s[i + 1..].chars().next() {
                if is_punct(next_ch) {
                    if i > last {
                        out.push_str(&s[last..i]);
                    }
                    out.push(next_ch);
                    i += 1 + next_ch.len_utf8();
                    last = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    if last < n {
        out.push_str(&s[last..]);
    }
    out
}

/// Replaces `/\\([\[\]])/g` with `$1` (`rules.other.outputLinkReplace`).
pub fn unescape_link_brackets(s: &str) -> String {
    if !s.as_bytes().contains(&b'\\') {
        return String::from(s);
    }
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n);
    let mut last = 0usize;
    let mut i = 0usize;
    while i + 1 < n {
        if bytes[i] == b'\\' && (bytes[i + 1] == b'[' || bytes[i + 1] == b']') {
            if i > last {
                out.push_str(&s[last..i]);
            }
            out.push(bytes[i + 1] as char);
            i += 2;
            last = i;
            continue;
        }
        i += 1;
    }
    if last < n {
        out.push_str(&s[last..]);
    }
    out
}

/// Normalises `\r\n` and `\r` to `\n` (`other.carriageReturn`).
pub fn normalize_newlines(src: &str) -> String {
    let bytes = src.as_bytes();
    if !bytes.contains(&b'\r') {
        return String::from(src);
    }
    let n = bytes.len();
    let mut out = String::with_capacity(n);
    let mut last = 0usize;
    let mut i = 0usize;
    while i < n {
        if bytes[i] == b'\r' {
            if i > last {
                out.push_str(&src[last..i]);
            }
            out.push('\n');
            if i + 1 < n && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
            last = i;
        } else {
            i += 1;
        }
    }
    if last < n {
        out.push_str(&src[last..]);
    }
    out
}

/// Pushes an unsigned decimal integer without pulling in `core::fmt`.
pub fn push_u64(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(buf[len] as char);
    }
}
