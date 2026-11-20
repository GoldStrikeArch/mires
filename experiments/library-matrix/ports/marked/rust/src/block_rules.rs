//! Deterministic native matchers for all block-level rules in `marked@18.1.1` (`src/rules.ts`).
//!
//! All matchers operate directly on `&[u8]` byte slices (UTF-8) with zero `&str` suffix reslicing,
//! avoiding O(N²) `$rt_bstr_bytes` materialization under `js-str=bytes` (`PERF-003`).

use alloc::string::String;

use crate::tokens::Mode;
use crate::unicode::{decode_utf8_at, is_all_js_space_bytes, js_space_len_at, trim_end_js};

#[inline]
pub fn line_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

#[inline]
fn leading_spaces_up_to(bytes: &[u8], pos: usize, max: usize) -> usize {
    let mut c = 0;
    while c < max && pos + c < bytes.len() && bytes[pos + c] == b' ' {
        c += 1;
    }
    c
}

/// Sorted list of the 62 CommonMark/marked block HTML tag names (`_tag`).
static HTML_BLOCK_TAGS: [&[u8]; 63] = [
    b"address",
    b"article",
    b"aside",
    b"base",
    b"basefont",
    b"blockquote",
    b"body",
    b"caption",
    b"center",
    b"col",
    b"colgroup",
    b"dd",
    b"details",
    b"dialog",
    b"dir",
    b"div",
    b"dl",
    b"dt",
    b"fieldset",
    b"figcaption",
    b"figure",
    b"footer",
    b"form",
    b"frame",
    b"frameset",
    b"h1",
    b"h2",
    b"h3",
    b"h4",
    b"h5",
    b"h6",
    b"head",
    b"header",
    b"hr",
    b"html",
    b"iframe",
    b"legend",
    b"li",
    b"link",
    b"main",
    b"menu",
    b"menuitem",
    b"meta",
    b"nav",
    b"noframes",
    b"ol",
    b"optgroup",
    b"option",
    b"p",
    b"param",
    b"search",
    b"section",
    b"summary",
    b"table",
    b"tbody",
    b"td",
    b"tfoot",
    b"th",
    b"thead",
    b"title",
    b"tr",
    b"track",
    b"ul",
];

static PEDANTIC_INLINE_TAGS: [&[u8]; 32] = [
    b"a",
    b"abbr",
    b"b",
    b"bdi",
    b"bdo",
    b"br",
    b"cite",
    b"code",
    b"data",
    b"del",
    b"dfn",
    b"em",
    b"i",
    b"img",
    b"ins",
    b"kbd",
    b"mark",
    b"q",
    b"rp",
    b"rt",
    b"ruby",
    b"s",
    b"samp",
    b"small",
    b"span",
    b"strong",
    b"sub",
    b"sup",
    b"time",
    b"u",
    b"var",
    b"wbr",
];

#[inline]
fn cmp_bytes(a: &[u8], b: &[u8]) -> i32 {
    let min_len = if a.len() < b.len() { a.len() } else { b.len() };
    let mut i = 0;
    while i < min_len {
        if a[i] != b[i] {
            return if a[i] < b[i] { -1 } else { 1 };
        }
        i += 1;
    }
    if a.len() < b.len() {
        -1
    } else if a.len() > b.len() {
        1
    } else {
        0
    }
}

fn binary_search_tag(table: &[&[u8]], key: &[u8]) -> bool {
    let mut lo = 0usize;
    let mut hi = table.len();
    while lo < hi {
        let mid = lo + ((hi - lo) >> 1);
        let c = cmp_bytes(table[mid], key);
        if c == 0 {
            return true;
        } else if c < 0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    false
}

fn is_html_block_tag(bytes: &[u8], case_insensitive: bool) -> bool {
    let len = bytes.len();
    if len == 0 || len > 10 {
        return false;
    }
    let mut lower = [0u8; 10];
    let mut i = 0;
    while i < len {
        let b = bytes[i];
        if case_insensitive {
            lower[i] = b.to_ascii_lowercase();
        } else {
            if b.is_ascii_uppercase() {
                return false;
            }
            lower[i] = b;
        }
        i += 1;
    }
    binary_search_tag(&HTML_BLOCK_TAGS, &lower[..len])
}

fn is_html_type1_tag(bytes: &[u8], case_insensitive: bool) -> bool {
    let len = bytes.len();
    if len < 3 || len > 8 {
        return false;
    }
    let mut lower = [0u8; 8];
    let mut i = 0;
    while i < len {
        let b = bytes[i];
        if case_insensitive {
            lower[i] = b.to_ascii_lowercase();
        } else {
            if b.is_ascii_uppercase() {
                return false;
            }
            lower[i] = b;
        }
        i += 1;
    }
    let k = &lower[..len];
    k == b"pre" || k == b"style" || k == b"script" || k == b"textarea"
}

fn is_pedantic_inline_tag(bytes: &[u8]) -> bool {
    binary_search_tag(&PEDANTIC_INLINE_TAGS, bytes)
}

#[inline]
fn starts_with_ascii_ci(bytes: &[u8], pos: usize, needle: &[u8]) -> bool {
    if pos + needle.len() > bytes.len() {
        return false;
    }
    let mut i = 0;
    while i < needle.len() {
        if bytes[pos + i].to_ascii_lowercase() != needle[i].to_ascii_lowercase() {
            return false;
        }
        i += 1;
    }
    true
}

/// Matches `block.newline`: `/^(?:[ \t]*(?:\n|$))+/`. Returns byte length from `pos` if > 0.
pub fn match_newline(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    let mut matched_end = pos;
    while i < n {
        let mut j = i;
        while j < n && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        if j < n && bytes[j] == b'\n' {
            i = j + 1;
            matched_end = i;
        } else if j == n {
            if j > i {
                matched_end = n;
            }
            break;
        } else {
            break;
        }
    }
    if matched_end > pos {
        Some(matched_end - pos)
    } else {
        None
    }
}

#[inline]
fn match_code_indent(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 4);
    if sp == 4 {
        return Some(pos + 4);
    }
    if pos + sp < n && bytes[pos + sp] == b'\t' {
        return Some(pos + sp + 1);
    }
    None
}

/// Matches `block.code` at `bytes[pos..]`, returning byte length from `pos`.
pub fn match_block_code(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    let mut any = false;
    while i < n {
        let Some(after_indent) = match_code_indent(bytes, i) else {
            break;
        };
        if after_indent >= n || bytes[after_indent] == b'\n' {
            break;
        }
        any = true;
        i = line_end(bytes, after_indent);
        if i < n && bytes[i] == b'\n' {
            i += 1;
            loop {
                let mut j = i;
                while j < n && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j < n && bytes[j] == b'\n' {
                    i = j + 1;
                } else if j == n {
                    i = n;
                    break;
                } else {
                    break;
                }
            }
        }
    }
    if any { Some(i - pos) } else { None }
}

fn match_closing_fence(bytes: &[u8], pos: usize, fence_char: u8, fence_len: usize) -> Option<usize> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let mut i = pos + sp;
    let mut count = 0;
    while i < n && bytes[i] == fence_char {
        i += 1;
        count += 1;
    }
    if count < fence_len {
        return None;
    }
    while i < n && (bytes[i] == b'~' || bytes[i] == b'`') {
        i += 1;
    }
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    if i == n || bytes[i] == b'\n' {
        Some(i)
    } else {
        None
    }
}

pub struct FencesSpan {
    pub raw_end: usize,
    pub info_start: usize,
    pub info_end: usize,
    pub body_start: usize,
    pub body_end: usize,
}

/// Matches `block.fences` at `bytes[pos..]`.
pub fn match_fences(bytes: &[u8], pos: usize) -> Option<FencesSpan> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let f_start = pos + sp;
    if f_start >= n {
        return None;
    }
    let ch = bytes[f_start];
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let mut fence_end = f_start;
    while fence_end < n && bytes[fence_end] == ch {
        fence_end += 1;
    }
    let fence_len = fence_end - f_start;
    if fence_len < 3 {
        return None;
    }
    let open_line_end = line_end(bytes, fence_end);
    if ch == b'`' && bytes[fence_end..open_line_end].contains(&b'`') {
        return None;
    }
    if open_line_end == n || open_line_end + 1 == n {
        return Some(FencesSpan {
            raw_end: n,
            info_start: fence_end,
            info_end: open_line_end,
            body_start: n,
            body_end: n,
        });
    }
    let body_start = open_line_end + 1;
    if let Some(close_end) = match_closing_fence(bytes, body_start, ch, fence_len) {
        return Some(FencesSpan {
            raw_end: close_end,
            info_start: fence_end,
            info_end: open_line_end,
            body_start,
            body_end: body_start,
        });
    }
    let mut cur = body_start;
    while cur < n {
        let le = line_end(bytes, cur);
        if le < n {
            if let Some(close_end) = match_closing_fence(bytes, le + 1, ch, fence_len) {
                return Some(FencesSpan {
                    raw_end: close_end,
                    info_start: fence_end,
                    info_end: open_line_end,
                    body_start,
                    body_end: le,
                });
            }
            cur = le + 1;
        } else {
            break;
        }
    }
    let body_end = if bytes[n - 1] == b'\n' && n > body_start {
        n - 1
    } else {
        n
    };
    Some(FencesSpan {
        raw_end: n,
        info_start: fence_end,
        info_end: open_line_end,
        body_start,
        body_end,
    })
}

pub fn is_fences_begin(bytes: &[u8], pos: usize, max_indent: usize) -> bool {
    let sp = leading_spaces_up_to(bytes, pos, max_indent);
    let i = pos + sp;
    if i + 3 > bytes.len() {
        return false;
    }
    (bytes[i] == b'`' && bytes[i + 1] == b'`' && bytes[i + 2] == b'`')
        || (bytes[i] == b'~' && bytes[i + 1] == b'~' && bytes[i + 2] == b'~')
}

fn is_fences_interrupt(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let i = pos + sp;
    if i + 3 > n {
        return false;
    }
    if bytes[i] == b'`' && bytes[i + 1] == b'`' && bytes[i + 2] == b'`' {
        let mut j = i + 3;
        while j < n && bytes[j] == b'`' {
            j += 1;
        }
        let le = line_end(bytes, j);
        return !bytes[j..le].contains(&b'`');
    }
    bytes[i] == b'~' && bytes[i + 1] == b'~' && bytes[i + 2] == b'~'
}

pub struct HeadingSpan {
    pub raw_end: usize,
    pub depth: usize,
    pub text_start: usize,
    pub text_end: usize,
}

/// Matches `block.heading` at `bytes[pos..]`.
pub fn match_heading(bytes: &[u8], pos: usize, pedantic: bool) -> Option<HeadingSpan> {
    let n = bytes.len();
    let sp = if pedantic {
        0
    } else {
        leading_spaces_up_to(bytes, pos, 3)
    };
    let h_start = pos + sp;
    let mut i = h_start;
    while i < n && i - h_start < 6 && bytes[i] == b'#' {
        i += 1;
    }
    let depth = i - h_start;
    if depth == 0 {
        return None;
    }
    if !pedantic && i < n && js_space_len_at(bytes, i) == 0 {
        return None;
    }
    let le = line_end(bytes, i);
    Some(HeadingSpan {
        raw_end: le,
        depth,
        text_start: i,
        text_end: le,
    })
}

fn is_heading_interrupt(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let start = pos + sp;
    let mut i = start;
    while i < n && i - start < 6 && bytes[i] == b'#' {
        i += 1;
    }
    if i == start {
        return false;
    }
    i == n || js_space_len_at(bytes, i) > 0
}

/// Checks pedantic paragraph heading interrupt: ` *#{1,6} *[^\n]`.
fn is_pedantic_heading_interrupt(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let mut i = pos;
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    let start = i;
    while i < n && i - start < 6 && bytes[i] == b'#' {
        i += 1;
    }
    let hashes = i - start;
    if hashes == 0 {
        return false;
    }
    // ` *#{1,6} *[^\n]`: if >= 2 `#`s, `#{1,5}` can backtrack 1 `#` to satisfy `[^\n]`;
    // if 1 `#`, ` *` can backtrack to 0 spaces so any non-`\n` char (including `' '`) satisfies `[^\n]`.
    hashes >= 2 || (i < n && bytes[i] != b'\n')
}

pub fn is_heading_begin(bytes: &[u8], pos: usize, max_indent: usize) -> bool {
    let sp = leading_spaces_up_to(bytes, pos, max_indent);
    pos + sp < bytes.len() && bytes[pos + sp] == b'#'
}

/// Matches `hr` at `bytes[pos..]`, returning end of line (rtrimmed of `\n`).
pub fn match_hr_at(bytes: &[u8], pos: usize, max_indent: usize) -> Option<usize> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, max_indent);
    let mut i = pos + sp;
    if i >= n {
        return None;
    }
    let marker = bytes[i];
    if marker != b'-' && marker != b'_' && marker != b'*' {
        return None;
    }
    let mut count = 0usize;
    while i < n {
        let b = bytes[i];
        if b == marker {
            count += 1;
            i += 1;
        } else if b == b' ' || b == b'\t' {
            i += 1;
        } else if b == b'\n' {
            break;
        } else {
            return None;
        }
    }
    if count >= 3 { Some(i) } else { None }
}

pub fn is_blockquote_begin(bytes: &[u8], pos: usize, max_indent: usize) -> bool {
    let sp = leading_spaces_up_to(bytes, pos, max_indent);
    pos + sp < bytes.len() && bytes[pos + sp] == b'>'
}

#[inline]
pub fn is_blockquote_at(bytes: &[u8], pos: usize) -> bool {
    let sp = leading_spaces_up_to(bytes, pos, 3);
    pos + sp < bytes.len() && bytes[pos + sp] == b'>'
}

fn match_bullet_at(bytes: &[u8], pos: usize, max_indent: usize) -> Option<usize> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, max_indent);
    let i = pos + sp;
    if i >= n {
        return None;
    }
    let b = bytes[i];
    if b == b'*' || b == b'+' || b == b'-' {
        return Some(i + 1);
    }
    if b.is_ascii_digit() {
        let mut j = i;
        while j < n && j - i < 9 && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j < n && (bytes[j] == b'.' || bytes[j] == b')') {
            return Some(j + 1);
        }
    }
    None
}

pub fn is_next_bullet(bytes: &[u8], pos: usize, max_indent: usize) -> bool {
    let Some(after) = match_bullet_at(bytes, pos, max_indent) else {
        return false;
    };
    after == bytes.len() || bytes[after] == b' ' || bytes[after] == b'\t' || bytes[after] == b'\n'
}

fn is_paragraph_list_interrupt(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let mut i = pos + sp;
    if i >= n {
        return false;
    }
    if bytes[i] == b'*' || bytes[i] == b'+' || bytes[i] == b'-' {
        i += 1;
    } else if bytes[i] == b'1' && i + 1 < n && (bytes[i + 1] == b'.' || bytes[i + 1] == b')') {
        i += 2;
    } else {
        return false;
    }
    let ws_start = i;
    while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    i > ws_start && i < n && bytes[i] != b'\n'
}

fn is_table_list_interrupt(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let mut i = pos + sp;
    if i >= n {
        return false;
    }
    if bytes[i] == b'*' || bytes[i] == b'+' || bytes[i] == b'-' {
        i += 1;
    } else if bytes[i] == b'1' && i + 1 < n && (bytes[i + 1] == b'.' || bytes[i + 1] == b')') {
        i += 2;
    } else {
        return false;
    }
    i < n && (bytes[i] == b' ' || bytes[i] == b'\t')
}

fn is_blockquote_list_interrupt(bytes: &[u8], pos: usize) -> bool {
    let Some(after) = match_bullet_at(bytes, pos, 3) else {
        return false;
    };
    after == bytes.len() || bytes[after] == b' ' || bytes[after] == b'\t' || bytes[after] == b'\n'
}

pub struct ListBulletSpan {
    pub raw_end: usize,
    pub bullet_end: usize,
    pub rest_start: usize,
    pub rest_end: usize,
}

pub fn match_list_start(bytes: &[u8], pos: usize) -> Option<ListBulletSpan> {
    let n = bytes.len();
    let after = match_bullet_at(bytes, pos, 3)?;
    if after == n {
        return Some(ListBulletSpan {
            raw_end: n,
            bullet_end: after,
            rest_start: n,
            rest_end: n,
        });
    }
    if bytes[after] == b'\n' {
        return Some(ListBulletSpan {
            raw_end: after + 1,
            bullet_end: after,
            rest_start: after,
            rest_end: after,
        });
    }
    if bytes[after] != b' ' && bytes[after] != b'\t' {
        return None;
    }
    let le = line_end(bytes, after);
    let raw_end = if le < n { le + 1 } else { n };
    Some(ListBulletSpan {
        raw_end,
        bullet_end: after,
        rest_start: after,
        rest_end: le,
    })
}

#[derive(Clone, Copy)]
pub enum ListMarkerSpec {
    Ordered(u8),
    Unordered(u8),
    PedanticAnyUnordered,
}

pub fn match_list_item(bytes: &[u8], pos: usize, spec: ListMarkerSpec) -> Option<ListBulletSpan> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let i = pos + sp;
    if i >= n {
        return None;
    }
    let after = match spec {
        ListMarkerSpec::Ordered(delim) => {
            if !bytes[i].is_ascii_digit() {
                return None;
            }
            let mut j = i;
            while j < n && j - i < 9 && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < n && bytes[j] == delim {
                j + 1
            } else {
                return None;
            }
        }
        ListMarkerSpec::Unordered(ch) => {
            if bytes[i] == ch {
                i + 1
            } else {
                return None;
            }
        }
        ListMarkerSpec::PedanticAnyUnordered => {
            if bytes[i] == b'*' || bytes[i] == b'+' || bytes[i] == b'-' {
                i + 1
            } else {
                return None;
            }
        }
    };
    if after == n {
        return Some(ListBulletSpan {
            raw_end: n,
            bullet_end: after,
            rest_start: n,
            rest_end: n,
        });
    }
    if bytes[after] == b'\n' {
        return Some(ListBulletSpan {
            raw_end: after + 1,
            bullet_end: after,
            rest_start: after,
            rest_end: after,
        });
    }
    if bytes[after] != b' ' && bytes[after] != b'\t' {
        return None;
    }
    let le = line_end(bytes, after);
    let raw_end = if le < n { le + 1 } else { n };
    Some(ListBulletSpan {
        raw_end,
        bullet_end: after,
        rest_start: after,
        rest_end: le,
    })
}

pub fn is_html_interrupt(
    bytes: &[u8],
    pos: usize,
    max_indent: usize,
    case_insensitive: bool,
) -> bool {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, max_indent);
    let i = pos + sp;
    if i + 2 > n || bytes[i] != b'<' {
        return false;
    }
    let b1 = bytes[i + 1];
    if b1 == b'?' {
        return true;
    }
    if b1 == b'!' {
        if i + 2 < n && bytes[i + 2].is_ascii_alphabetic() {
            return true;
        }
        if i + 4 <= n && &bytes[i..i + 4] == b"<!--" {
            return true;
        }
        if case_insensitive {
            if starts_with_ascii_ci(bytes, i, b"<![cdata[") {
                return true;
            }
        } else if i + 9 <= n && &bytes[i..i + 9] == b"<![CDATA[" {
            return true;
        }
        return false;
    }
    let is_close = b1 == b'/';
    let name_start = if is_close { i + 2 } else { i + 1 };
    let mut name_end = name_start;
    while name_end < n && bytes[name_end].is_ascii_alphanumeric() {
        name_end += 1;
    }
    if name_end == name_start {
        return false;
    }
    let name = &bytes[name_start..name_end];
    if !is_close && is_html_type1_tag(name, case_insensitive) {
        if name_end == n || bytes[name_end] == b'>' || js_space_len_at(bytes, name_end) > 0 {
            return true;
        }
    }
    if is_html_block_tag(name, case_insensitive) {
        if name_end == n {
            return case_insensitive;
        }
        let next = bytes[name_end];
        if next == b' ' || next == b'>' {
            return true;
        }
        if !case_insensitive && next == b'\n' {
            return true;
        }
        if next == b'/' && name_end + 1 < n && bytes[name_end + 1] == b'>' {
            return true;
        }
    }
    false
}

fn scan_to_blank_line_or_eof(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    let mut i = start;
    while i < n {
        if bytes[i] == b'\n' {
            let mut j = i;
            let mut blank_count = 0usize;
            while j < n && bytes[j] == b'\n' {
                let mut k = j + 1;
                while k < n && (bytes[k] == b' ' || bytes[k] == b'\t') {
                    k += 1;
                }
                if k < n && bytes[k] == b'\n' {
                    blank_count += 1;
                    j = k;
                } else {
                    break;
                }
            }
            if blank_count > 0 {
                return j + 1;
            }
        }
        i += 1;
    }
    n
}

fn match_block_html_attribute(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    if i == pos || i >= n {
        return None;
    }
    let b0 = bytes[i];
    if !(b0.is_ascii_alphabetic() || b0 == b':' || b0 == b'_') {
        return None;
    }
    i += 1;
    while i < n {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b':' || b == b'-' {
            i += 1;
        } else {
            break;
        }
    }
    let save = i;
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    if i < n && bytes[i] == b'=' {
        i += 1;
        while i < n && bytes[i] == b' ' {
            i += 1;
        }
        if i < n && bytes[i] == b'"' {
            i += 1;
            while i < n && bytes[i] != b'"' && bytes[i] != b'\n' {
                i += 1;
            }
            if i < n && bytes[i] == b'"' {
                return Some(i + 1);
            }
            return Some(save);
        }
        if i < n && bytes[i] == b'\'' {
            i += 1;
            while i < n && bytes[i] != b'\'' && bytes[i] != b'\n' {
                i += 1;
            }
            if i < n && bytes[i] == b'\'' {
                return Some(i + 1);
            }
            return Some(save);
        }
        let val_start = i;
        while i < n {
            let b = bytes[i];
            if matches!(b, b'"' | b'\'' | b'=' | b'<' | b'>' | b'`') {
                break;
            }
            if js_space_len_at(bytes, i) > 0 {
                break;
            }
            let (_, clen) = decode_utf8_at(bytes, i);
            i += clen;
        }
        if i > val_start {
            return Some(i);
        }
        return Some(save);
    }
    Some(save)
}

/// Matches `block.html` (normal/GFM) at `bytes[pos..]`. Returns `raw_end` (index in `bytes`).
pub fn match_html_normal(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let i = pos + sp;
    if i + 2 > n || bytes[i] != b'<' {
        return None;
    }

    // Type 1: `<(script|pre|style|textarea)[\s>][\s\S]*?(?:</\1>[^\n]*\n*|$)` (ci)
    if bytes[i + 1].is_ascii_alphabetic() {
        let mut ne = i + 1;
        while ne < n && bytes[ne].is_ascii_alphabetic() {
            ne += 1;
        }
        let tag = &bytes[i + 1..ne];
        if is_html_type1_tag(tag, true) && ne < n {
            if bytes[ne] == b'>' || js_space_len_at(bytes, ne) > 0 {
                let mut scan = ne + 1;
                while scan < n {
                    if bytes[scan] == b'<'
                        && scan + 2 + tag.len() < n
                        && bytes[scan + 1] == b'/'
                        && starts_with_ascii_ci(bytes, scan + 2, tag)
                        && bytes[scan + 2 + tag.len()] == b'>'
                    {
                        let mut end = line_end(bytes, scan + 3 + tag.len());
                        while end < n && bytes[end] == b'\n' {
                            end += 1;
                        }
                        return Some(end);
                    }
                    scan += 1;
                }
                return Some(n);
            }
        }
    }

    // Type 2: `<!--(?:-?>|[\s\S]*?(?:-->|$))[^\n]*(\n+|$)`
    if i + 4 <= n && &bytes[i..i + 4] == b"<!--" {
        let after = if i + 4 < n && bytes[i + 4] == b'>' {
            i + 5
        } else if i + 5 < n && bytes[i + 4] == b'-' && bytes[i + 5] == b'>' {
            i + 6
        } else {
            let mut j = i + 4;
            let mut found = n;
            while j + 2 < n {
                if bytes[j] == b'-' && bytes[j + 1] == b'-' && bytes[j + 2] == b'>' {
                    found = j + 3;
                    break;
                }
                j += 1;
            }
            found
        };
        let mut end = line_end(bytes, after);
        while end < n && bytes[end] == b'\n' {
            end += 1;
        }
        return Some(end);
    }

    // Type 3: `<\?`
    if bytes[i + 1] == b'?' {
        let mut j = i + 2;
        while j + 1 < n {
            if bytes[j] == b'?' && bytes[j + 1] == b'>' {
                let mut end = line_end(bytes, j + 2);
                while end < n && bytes[end] == b'\n' {
                    end += 1;
                }
                return Some(end);
            }
            j += 1;
        }
        return Some(n);
    }

    // Type 4: `<![A-Za-z]`
    if bytes[i + 1] == b'!' && i + 2 < n && bytes[i + 2].is_ascii_alphabetic() {
        let mut j = i + 3;
        while j < n {
            if bytes[j] == b'>' {
                let mut end = line_end(bytes, j + 1);
                while end < n && bytes[end] == b'\n' {
                    end += 1;
                }
                return Some(end);
            }
            j += 1;
        }
        return Some(n);
    }

    // Type 5: `<![CDATA[`
    if starts_with_ascii_ci(bytes, i, b"<![cdata[") {
        let mut j = i + 9;
        while j + 2 < n {
            if bytes[j] == b']' && bytes[j + 1] == b']' && bytes[j + 2] == b'>' {
                let mut end = line_end(bytes, j + 3);
                while end < n && bytes[end] == b'\n' {
                    end += 1;
                }
                return Some(end);
            }
            j += 1;
        }
        return Some(n);
    }

    // Type 6: `</?(tag)(?: +|\n|/?>)`
    {
        let is_close = bytes[i + 1] == b'/';
        let ns = if is_close { i + 2 } else { i + 1 };
        let mut ne = ns;
        while ne < n && bytes[ne].is_ascii_alphanumeric() {
            ne += 1;
        }
        if ne > ns && is_html_block_tag(&bytes[ns..ne], true) && ne < n {
            let after_delim = if bytes[ne] == b' ' {
                let mut k = ne;
                while k < n && bytes[k] == b' ' {
                    k += 1;
                }
                Some(k)
            } else if bytes[ne] == b'\n' || bytes[ne] == b'>' {
                Some(ne + 1)
            } else if bytes[ne] == b'/' && ne + 1 < n && bytes[ne + 1] == b'>' {
                Some(ne + 2)
            } else {
                None
            };
            if let Some(after) = after_delim {
                return Some(scan_to_blank_line_or_eof(bytes, after));
            }
        }
    }

    // Type 7 open
    if bytes[i + 1].is_ascii_alphabetic() {
        let ns = i + 1;
        let mut ne = ns + 1;
        while ne < n && (bytes[ne].is_ascii_alphanumeric() || bytes[ne] == b'-') {
            ne += 1;
        }
        if !is_html_type1_tag(&bytes[ns..ne], true) {
            let mut cur = ne;
            loop {
                let mut c = cur;
                while c < n && bytes[c] == b' ' {
                    c += 1;
                }
                if c < n && bytes[c] == b'/' {
                    c += 1;
                }
                if c < n && bytes[c] == b'>' {
                    let after_gt = c + 1;
                    let mut w = after_gt;
                    while w < n && (bytes[w] == b' ' || bytes[w] == b'\t') {
                        w += 1;
                    }
                    if w == n || bytes[w] == b'\n' {
                        return Some(scan_to_blank_line_or_eof(bytes, after_gt));
                    }
                }
                if let Some(next_cur) = match_block_html_attribute(bytes, cur) {
                    cur = next_cur;
                } else {
                    break;
                }
            }
        }
    }

    // Type 7 close
    if bytes[i + 1] == b'/' && i + 2 < n && bytes[i + 2].is_ascii_alphabetic() {
        let mut cur = i + 3;
        while cur < n && (bytes[cur].is_ascii_alphanumeric() || bytes[cur] == b'-') {
            cur += 1;
        }
        while cur < n {
            let sl = js_space_len_at(bytes, cur);
            if sl > 0 {
                cur += sl;
            } else {
                break;
            }
        }
        if cur < n && bytes[cur] == b'>' {
            let after_gt = cur + 1;
            let mut w = after_gt;
            while w < n && (bytes[w] == b' ' || bytes[w] == b'\t') {
                w += 1;
            }
            if w == n || bytes[w] == b'\n' {
                return Some(scan_to_blank_line_or_eof(bytes, after_gt));
            }
        }
    }

    None
}

fn match_pedantic_html_tag_name(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut end = pos;
    while end < n && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if end == pos || is_pedantic_inline_tag(&bytes[pos..end]) {
        return None;
    }
    if end < n && bytes[end] == b':' {
        return None;
    }
    let mut j = end;
    while j < n {
        let b = bytes[j];
        if b == b'@' {
            return None;
        }
        if b.is_ascii_alphanumeric() || b == b'_' || js_space_len_at(bytes, j) > 0 {
            break;
        }
        let (_, clen) = decode_utf8_at(bytes, j);
        j += clen;
    }
    Some(end)
}

/// Matches `blockPedantic.html` at `bytes[pos..]`, returning `raw_end`.
pub fn match_html_pedantic(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    if i + 2 > n || bytes[i] != b'<' {
        return None;
    }

    // Alt 1: `comment *(?:\n|\s*$)` where `_comment = /<!--(?:-?>|[\s\S]*?(?:-->|$))/`
    if i + 4 <= n && &bytes[i..i + 4] == b"<!--" {
        let check_after = |after: usize| -> Option<usize> {
            let mut j = after;
            while j < n && bytes[j] == b' ' {
                j += 1;
            }
            if j < n && bytes[j] == b'\n' {
                return Some(j + 1);
            }
            if is_all_js_space_bytes(bytes, j, n) {
                return Some(n);
            }
            None
        };
        if i + 4 < n && bytes[i + 4] == b'>' {
            if let Some(end) = check_after(i + 5) {
                return Some(end);
            }
        } else if i + 5 < n && bytes[i + 4] == b'-' && bytes[i + 5] == b'>' {
            if let Some(end) = check_after(i + 6) {
                return Some(end);
            }
        }
        // `[\s\S]*?(?:-->|$)` tries each `-->` in order and finally `$` (EOF)!
        let mut j = i + 4;
        while j + 2 < n {
            if bytes[j] == b'-' && bytes[j + 1] == b'-' && bytes[j + 2] == b'>' {
                if let Some(end) = check_after(j + 3) {
                    return Some(end);
                }
            }
            j += 1;
        }
        // `$` alternative of `(?:-->|$)` always satisfies ` *(?:\n|\s*$)` at EOF!
        return Some(n);
    }

    let name_end = match_pedantic_html_tag_name(bytes, i + 1)?;
    let tag = &bytes[i + 1..name_end];

    // Alt 2: `<(tag)[\s\S]+?</\1> *(?:\n{2,}|\s*$)`
    {
        let mut scan = name_end + 1;
        while scan < n {
            if bytes[scan] == b'<'
                && scan + 2 + tag.len() < n
                && bytes[scan + 1] == b'/'
                && &bytes[scan + 2..scan + 2 + tag.len()] == tag
                && bytes[scan + 2 + tag.len()] == b'>'
            {
                let mut end = scan + 3 + tag.len();
                while end < n && bytes[end] == b' ' {
                    end += 1;
                }
                if end + 1 < n && bytes[end] == b'\n' && bytes[end + 1] == b'\n' {
                    while end < n && bytes[end] == b'\n' {
                        end += 1;
                    }
                    return Some(end);
                }
                if is_all_js_space_bytes(bytes, end, n) {
                    return Some(n);
                }
            }
            scan += 1;
        }
    }

    // Alt 3: `<tag(?:"[^"]*"|'[^']*'|\s[^'"/>\s]*)*?/?> *(?:\n{2,}|\s*$)`
    {
        let mut cur = name_end;
        loop {
            let mut c = cur;
            if c < n && bytes[c] == b'/' {
                c += 1;
            }
            if c < n && bytes[c] == b'>' {
                let mut end = c + 1;
                while end < n && bytes[end] == b' ' {
                    end += 1;
                }
                if end + 1 < n && bytes[end] == b'\n' && bytes[end + 1] == b'\n' {
                    while end < n && bytes[end] == b'\n' {
                        end += 1;
                    }
                    return Some(end);
                }
                if is_all_js_space_bytes(bytes, end, n) {
                    return Some(n);
                }
            }
            if cur >= n {
                break;
            }
            if bytes[cur] == b'"' {
                let mut j = cur + 1;
                while j < n && bytes[j] != b'"' {
                    j += 1;
                }
                if j < n {
                    cur = j + 1;
                    continue;
                }
                break;
            } else if bytes[cur] == b'\'' {
                let mut j = cur + 1;
                while j < n && bytes[j] != b'\'' {
                    j += 1;
                }
                if j < n {
                    cur = j + 1;
                    continue;
                }
                break;
            } else {
                let sl = js_space_len_at(bytes, cur);
                if sl > 0 {
                    cur += sl;
                    while cur < n {
                        let b = bytes[cur];
                        if b == b'\'' || b == b'"' || b == b'/' || b == b'>' {
                            break;
                        }
                        if js_space_len_at(bytes, cur) > 0 {
                            break;
                        }
                        let (_, clen) = decode_utf8_at(bytes, cur);
                        cur += clen;
                    }
                    continue;
                }
                break;
            }
        }
    }

    None
}

pub struct DefSpan {
    pub raw_end: usize,
    pub label_start: usize,
    pub label_end: usize,
    pub href_start: usize,
    pub href_end: usize,
    pub title: Option<(usize, usize)>,
}

/// Parses `_blockLabel`: `(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+` at `bytes[pos..]`, returning `]` index.
pub fn match_block_label(bytes: &[u8], pos: usize, max_items: Option<usize>) -> Option<usize> {
    let n = bytes.len();
    let mut w = pos;
    while w < n {
        let sl = js_space_len_at(bytes, w);
        if sl > 0 {
            w += sl;
        } else {
            break;
        }
    }
    if w < n && bytes[w] == b']' {
        return None;
    }
    let mut i = pos;
    let mut items = 0usize;
    while i < n {
        if let Some(max) = max_items {
            if items >= max {
                break;
            }
        }
        let b = bytes[i];
        if b == b'[' || b == b']' {
            break;
        }
        if b == b'\\' {
            if i + 1 >= n {
                return None;
            }
            let (_, clen) = decode_utf8_at(bytes, i + 1);
            i += 1 + clen;
            items += 1;
        } else {
            let (_, clen) = decode_utf8_at(bytes, i);
            i += clen;
            items += 1;
        }
    }
    if items > 0 && i < n && bytes[i] == b']' {
        Some(i)
    } else {
        None
    }
}

fn match_def_title(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    if pos >= n {
        return None;
    }
    match bytes[pos] {
        b'"' => {
            let mut i = pos + 1;
            while i < n {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i < n && bytes[i] == b'"' {
                        i += 1;
                    }
                } else if bytes[i] == b'"' {
                    return Some(i + 1);
                } else {
                    i += 1;
                }
            }
            None
        }
        b'\'' => {
            let mut i = pos + 1;
            while i < n && bytes[i] != b'\'' && bytes[i] != b'\n' {
                i += 1;
            }
            while i < n && bytes[i] == b'\n' {
                let after_nl = i + 1;
                if after_nl < n && bytes[after_nl] == b'\'' {
                    return Some(after_nl + 1);
                }
                let mut j = after_nl;
                while j < n && bytes[j] != b'\'' && bytes[j] != b'\n' {
                    j += 1;
                }
                if j == after_nl {
                    return None;
                }
                i = j;
            }
            if i < n && bytes[i] == b'\'' {
                Some(i + 1)
            } else {
                None
            }
        }
        b'(' => {
            let mut i = pos + 1;
            while i < n {
                if bytes[i] == b'(' {
                    return None;
                }
                if bytes[i] == b')' {
                    return Some(i + 1);
                }
                i += 1;
            }
            None
        }
        _ => None,
    }
}

fn match_def_end(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    if i == n || bytes[i] == b'\n' {
        Some(i)
    } else {
        None
    }
}

pub fn match_def(bytes: &[u8], pos: usize, pedantic: bool) -> Option<DefSpan> {
    let n = bytes.len();
    if pedantic {
        let mut i = pos;
        while i < n && bytes[i] == b' ' {
            i += 1;
        }
        if i >= n || bytes[i] != b'[' {
            return None;
        }
        let label_start = i + 1;
        let mut label_end = label_start;
        while label_end < n && bytes[label_end] != b']' {
            label_end += 1;
        }
        if label_end == label_start || label_end + 1 >= n || bytes[label_end + 1] != b':' {
            return None;
        }
        i = label_end + 2;
        while i < n && bytes[i] == b' ' {
            i += 1;
        }
        if i < n && bytes[i] == b'<' {
            i += 1;
        }
        let href_start = i;
        while i < n && bytes[i] != b'>' && js_space_len_at(bytes, i) == 0 {
            let (_, clen) = decode_utf8_at(bytes, i);
            i += clen;
        }
        let href_end = i;
        if href_end == href_start {
            return None;
        }
        if i < n && bytes[i] == b'>' {
            i += 1;
        }
        let after_href = i;
        let le = line_end(bytes, after_href);
        if after_href < le && bytes[after_href] == b' ' {
            let mut ts = after_href;
            while ts < le && bytes[ts] == b' ' {
                ts += 1;
            }
            if ts < le && (bytes[ts] == b'"' || bytes[ts] == b'(') {
                let mut te = le;
                while te > ts && bytes[te - 1] == b' ' {
                    te -= 1;
                }
                if te >= ts + 3 && (bytes[te - 1] == b'"' || bytes[te - 1] == b')') {
                    return Some(DefSpan {
                        raw_end: le,
                        label_start,
                        label_end,
                        href_start,
                        href_end,
                        title: Some((ts, te)),
                    });
                }
            }
        }
        let raw_end = match_def_end(bytes, after_href)?;
        return Some(DefSpan {
            raw_end,
            label_start,
            label_end,
            href_start,
            href_end,
            title: None,
        });
    }

    let sp = leading_spaces_up_to(bytes, pos, 3);
    let open = pos + sp;
    if open >= n || bytes[open] != b'[' {
        return None;
    }
    let label_start = open + 1;
    let label_end = match_block_label(bytes, label_start, None)?;
    if label_end + 1 >= n || bytes[label_end + 1] != b':' {
        return None;
    }
    let mut i = label_end + 2;
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    if i < n && bytes[i] == b'\n' {
        i += 1;
        while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
    }
    if i >= n {
        return None;
    }
    let href_start = i;
    if bytes[i] == b'<' {
        // `<.*?>` is lazy and backtracks across `>` on the same line until the
        // optional title and line-end anchor match.
        let mut j = i + 1;
        while j < n && bytes[j] != b'\n' {
            if bytes[j] == b'>' {
                let href_end = j + 1;
                if let Some(span) =
                    match_def_tail(bytes, label_start, label_end, href_start, href_end)
                {
                    return Some(span);
                }
            }
            j += 1;
        }
        return None;
    }

    if js_space_len_at(bytes, i) > 0 {
        return None;
    }
    let mut j = i;
    while j < n && js_space_len_at(bytes, j) == 0 {
        let (_, clen) = decode_utf8_at(bytes, j);
        j += clen;
    }
    if j == i {
        return None;
    }
    match_def_tail(bytes, label_start, label_end, href_start, j)
}

fn match_def_tail(
    bytes: &[u8],
    label_start: usize,
    label_end: usize,
    href_start: usize,
    href_end: usize,
) -> Option<DefSpan> {
    let n = bytes.len();
    let mut t = href_end;
    let mut has_space = false;
    while t < n && bytes[t] == b' ' {
        t += 1;
        has_space = true;
    }
    let mut has_nl = false;
    if t < n && bytes[t] == b'\n' {
        t += 1;
        has_nl = true;
        while t < n && (bytes[t] == b' ' || bytes[t] == b'\t') {
            t += 1;
        }
    }
    if has_space || has_nl {
        if let Some(title_end) = match_def_title(bytes, t) {
            if let Some(raw_end) = match_def_end(bytes, title_end) {
                return Some(DefSpan {
                    raw_end,
                    label_start,
                    label_end,
                    href_start,
                    href_end,
                    title: Some((t, title_end)),
                });
            }
        }
    }

    let raw_end = match_def_end(bytes, href_end)?;
    Some(DefSpan {
        raw_end,
        label_start,
        label_end,
        href_start,
        href_end,
        title: None,
    })
}

pub struct TableSpan {
    pub raw_end: usize,
    pub header_start: usize,
    pub header_end: usize,
    pub align_start: usize,
    pub align_end: usize,
    pub body: Option<(usize, usize)>,
}

fn match_table_align_line(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let align_start = pos + sp;
    let mut i = align_start;
    if i < n && bytes[i] == b'|' {
        i += 1;
        while i < n && bytes[i] == b' ' {
            i += 1;
        }
    }
    if i < n && bytes[i] == b':' {
        i += 1;
    }
    let d_start = i;
    while i < n && bytes[i] == b'-' {
        i += 1;
    }
    if i == d_start {
        return None;
    }
    if i < n && bytes[i] == b':' {
        i += 1;
    }
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    while i < n && bytes[i] == b'|' {
        let save = i;
        i += 1;
        while i < n && bytes[i] == b' ' {
            i += 1;
        }
        if i == n || bytes[i] == b'\n' {
            return Some((align_start, i));
        }
        if bytes[i] == b':' {
            i += 1;
        }
        let ds = i;
        while i < n && bytes[i] == b'-' {
            i += 1;
        }
        if i == ds {
            i = save;
            break;
        }
        if i < n && bytes[i] == b':' {
            i += 1;
        }
        while i < n && bytes[i] == b' ' {
            i += 1;
        }
    }
    if i < n && bytes[i] == b'|' {
        i += 1;
        while i < n && bytes[i] == b' ' {
            i += 1;
        }
    }
    if i == n || bytes[i] == b'\n' {
        Some((align_start, i))
    } else {
        None
    }
}

pub fn match_table_at(bytes: &[u8], pos: usize) -> Option<TableSpan> {
    let n = bytes.len();
    let mut i = pos;
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    if i >= n || bytes[i] == b'\n' {
        return None;
    }
    let header_start = i;
    let header_end = line_end(bytes, header_start);
    if header_end >= n {
        return None;
    }
    let (align_start, align_end) = match_table_align_line(bytes, header_end + 1)?;
    if align_end == n {
        return Some(TableSpan {
            raw_end: align_end,
            header_start,
            header_end,
            align_start,
            align_end,
            body: None,
        });
    }

    let body_start = align_end + 1;
    let mut cur = body_start;
    while cur < n {
        let mut s = cur;
        while s < n && bytes[s] == b' ' {
            s += 1;
        }
        if s < n && bytes[s] == b'\n' {
            break;
        }
        if match_hr_at(bytes, cur, 3).is_some()
            || is_heading_interrupt(bytes, cur)
            || is_blockquote_at(bytes, cur)
            || match_code_indent(bytes, cur).is_some_and(|a| a < n && bytes[a] != b'\n')
            || is_fences_interrupt(bytes, cur)
            || is_table_list_interrupt(bytes, cur)
            || is_html_interrupt(bytes, cur, 3, false)
        {
            break;
        }
        let le = line_end(bytes, cur);
        cur = if le < n { le + 1 } else { n };
    }
    let mut raw_end = cur;
    while raw_end > pos && bytes[raw_end - 1] == b'\n' {
        raw_end -= 1;
    }
    Some(TableSpan {
        raw_end,
        header_start,
        header_end,
        align_start,
        align_end,
        body: Some((body_start, cur)),
    })
}

fn match_setext_underline(bytes: &[u8], pos: usize) -> Option<(u8, usize)> {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let mut i = pos + sp;
    if i >= n {
        return None;
    }
    let ch = bytes[i];
    if ch != b'=' && ch != b'-' {
        return None;
    }
    while i < n && bytes[i] == ch {
        i += 1;
    }
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    if i == n || bytes[i] == b'\n' {
        Some((ch, i))
    } else {
        None
    }
}

fn is_lheading_html_at(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let i = pos + sp;
    if i >= n || bytes[i] != b'<' {
        return false;
    }
    let mut j = i + 1;
    while j < n && bytes[j] != b'\n' && bytes[j] != b'>' {
        j += 1;
    }
    j > i + 1 && j + 1 < n && bytes[j] == b'>' && bytes[j + 1] == b'\n'
}

fn is_lheading_table_at(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let sp = leading_spaces_up_to(bytes, pos, 3);
    let mut i = pos + sp;
    let mut pipes = 0usize;
    if i < n && bytes[i] == b'|' {
        i += 1;
    }
    while i < n && bytes[i] != b'\n' {
        let b = bytes[i];
        if b == b'|' {
            pipes += 1;
        } else if b != b':' && b != b'-' && b != b' ' {
            return false;
        }
        i += 1;
    }
    i < n && bytes[i] == b'\n' && pipes >= 1
}

fn is_lheading_bull_at(bytes: &[u8], pos: usize) -> bool {
    match_bullet_at(bytes, pos, 3).is_some_and(|after| after < bytes.len() && bytes[after] == b' ')
}

pub struct LHeadingSpan {
    pub raw_end: usize,
    pub depth: usize,
    pub text_start: usize,
    pub text_end: usize,
}

pub fn match_lheading_at(bytes: &[u8], pos: usize, mode: Mode) -> Option<LHeadingSpan> {
    let n = bytes.len();
    if pos >= n {
        return None;
    }
    if mode.is_pedantic() {
        let le = line_end(bytes, pos);
        if le == pos || le >= n {
            return None;
        }
        let (ch, u_end) = match_setext_underline(bytes, le + 1)?;
        return Some(LHeadingSpan {
            raw_end: u_end,
            depth: if ch == b'=' { 1 } else { 2 },
            text_start: pos,
            text_end: le,
        });
    }

    let is_gfm = mode.is_gfm();
    if is_lheading_bull_at(bytes, pos)
        || match_code_indent(bytes, pos).is_some()
        || is_fences_begin(bytes, pos, 3)
        || is_blockquote_at(bytes, pos)
        || is_heading_interrupt(bytes, pos)
        || is_lheading_html_at(bytes, pos)
        || (is_gfm && is_lheading_table_at(bytes, pos))
    {
        return None;
    }

    let mut cur = pos;
    loop {
        let le = line_end(bytes, cur);
        if le >= n {
            return None;
        }
        let next_start = le + 1;
        if le > pos {
            if let Some((ch, u_end)) = match_setext_underline(bytes, next_start) {
                return Some(LHeadingSpan {
                    raw_end: u_end,
                    depth: if ch == b'=' { 1 } else { 2 },
                    text_start: pos,
                    text_end: le,
                });
            }
        }
        let next_le = line_end(bytes, next_start);
        if next_le < n && is_all_js_space_bytes(bytes, next_start, next_le) {
            return None;
        }
        if is_lheading_bull_at(bytes, next_start)
            || is_fences_begin(bytes, next_start, 3)
            || is_blockquote_at(bytes, next_start)
            || is_heading_interrupt(bytes, next_start)
            || match_hr_at(bytes, next_start, 3).is_some()
            || is_lheading_html_at(bytes, next_start)
            || (is_gfm && is_lheading_table_at(bytes, next_start))
        {
            return None;
        }
        cur = next_start;
    }
}

#[inline]
fn is_space_tab_blank_line(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let mut i = pos;
    while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    i > pos && i < n && bytes[i] == b'\n'
}

/// Matches `paragraph` (or `blockquoteParagraph`) at `bytes[pos..]`, returning `end` index.
pub fn match_paragraph_at(
    bytes: &[u8],
    pos: usize,
    mode: Mode,
    for_blockquote: bool,
) -> Option<usize> {
    let n = bytes.len();
    if pos >= n || bytes[pos] == b'\n' {
        return None;
    }
    let mut cur = line_end(bytes, pos);
    while cur < n && bytes[cur] == b'\n' {
        let next = cur + 1;
        if next >= n || bytes[next] == b'\n' {
            break;
        }
        if is_space_tab_blank_line(bytes, next)
            || match_hr_at(bytes, next, 3).is_some()
            || is_blockquote_at(bytes, next)
        {
            break;
        }
        if !for_blockquote && mode.is_pedantic() {
            if is_pedantic_heading_interrupt(bytes, next)
                || match_lheading_at(bytes, next, Mode::Normal).is_some()
            {
                break;
            }
        } else {
            if is_heading_interrupt(bytes, next)
                || is_fences_interrupt(bytes, next)
                || is_html_interrupt(bytes, next, 3, false)
            {
                break;
            }
            if for_blockquote {
                if is_blockquote_list_interrupt(bytes, next) {
                    break;
                }
            } else {
                if is_paragraph_list_interrupt(bytes, next) {
                    break;
                }
                if mode.is_gfm() && match_table_at(bytes, next).is_some() {
                    break;
                }
            }
        }
        cur = line_end(bytes, next);
    }
    Some(cur)
}

/// Matches `block.blockquote` at `bytes[pos..]`, returning `end` index.
pub fn match_blockquote(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    let mut any = false;
    while i < n {
        let sp = leading_spaces_up_to(bytes, i, 3);
        let gt = i + sp;
        if gt >= n || bytes[gt] != b'>' {
            break;
        }
        any = true;
        let mut body = gt + 1;
        if body < n && bytes[body] == b' ' {
            body += 1;
        }
        if let Some(pend) = match_paragraph_at(bytes, body, Mode::Normal, true) {
            i = pend;
        } else {
            i = line_end(bytes, body);
        }
        if i < n && bytes[i] == b'\n' {
            i += 1;
        }
    }
    if any { Some(i) } else { None }
}

pub fn strip_blockquote_markers(chunk: &str) -> String {
    let mut out = String::with_capacity(chunk.len());
    let mut first = true;
    for line in chunk.split('\n') {
        if !first {
            out.push('\n');
        }
        let lb = line.as_bytes();
        if !first {
            let sp = leading_spaces_up_to(lb, 0, 3);
            if sp < lb.len() && (lb[sp] == b'=' || lb[sp] == b'-') {
                let ch = lb[sp];
                let mut k = sp;
                while k < lb.len() && lb[k] == ch {
                    k += 1;
                }
                while k < lb.len() && lb[k] == b' ' {
                    k += 1;
                }
                if k == lb.len() {
                    out.push_str("    ");
                    out.push_str(&line[sp..]);
                    first = false;
                    continue;
                }
            }
        }
        first = false;
        let sp = leading_spaces_up_to(lb, 0, 3);
        if sp < lb.len() && lb[sp] == b'>' {
            let mut after = sp + 1;
            if after < lb.len() && (lb[after] == b' ' || lb[after] == b'\t') {
                after += 1;
            }
            out.push_str(&line[after..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

pub fn strip_one_blockquote_prefix(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        let lb = line.as_bytes();
        let sp = leading_spaces_up_to(lb, 0, 3);
        if sp < lb.len() && lb[sp] == b'>' {
            let mut after = sp + 1;
            if after < lb.len() && (lb[after] == b' ' || lb[after] == b'\t') {
                after += 1;
            }
            out.push_str(&line[after..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

pub fn has_double_blank_line_end(bytes: &[u8]) -> bool {
    let mut i = bytes.len();
    while i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != b'\n' {
        return false;
    }
    i -= 1;
    while i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
        i -= 1;
    }
    i > 0 && bytes[i - 1] == b'\n'
}

pub fn has_any_line(s: &str) -> bool {
    let mut count = 0;
    for &b in s.as_bytes() {
        if b == b'\n' {
            count += 1;
            if count >= 2 {
                return true;
            }
        }
    }
    false
}

pub fn is_list_task(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 5 || bytes[0] != b'[' || bytes[2] != b']' {
        return false;
    }
    if bytes[1] != b' ' && bytes[1] != b'x' && bytes[1] != b'X' {
        return false;
    }
    let mut i = 3;
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    i > 3 && i < bytes.len() && js_space_len_at(bytes, i) == 0
}

pub fn strip_list_task(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'['
        && (bytes[1] == b' ' || bytes[1] == b'x' || bytes[1] == b'X')
        && bytes[2] == b']'
        && bytes[3] == b' '
    {
        let mut i = 3;
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        return &s[i..];
    }
    s
}

pub fn find_list_task_checkbox(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'['
            && (bytes[i + 1] == b' ' || bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
            && bytes[i + 2] == b']'
        {
            return Some(&s[i..i + 3]);
        }
        i += 1;
    }
    None
}

pub fn replace_pedantic_list_nesting(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut sp = 0;
    while sp < bytes.len() && bytes[sp] == b' ' {
        sp += 1;
    }
    if sp >= 1 && sp < bytes.len() {
        let mut k = if sp < 4 { sp } else { 4 };
        while k >= 1 {
            if (sp - k) % 4 == 0 {
                let mut out = String::with_capacity(line.len() + 2);
                out.push_str("  ");
                out.push_str(&line[k..]);
                return out;
            }
            k -= 1;
        }
    }
    String::from(line)
}

pub fn trim_table_row_blank_line(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
        i -= 1;
    }
    if i > 0 && bytes[i - 1] == b'\n' {
        &s[..i - 1]
    } else {
        s
    }
}

pub fn trim_table_align_chars(s: &str) -> &str {
    let s = s.strip_prefix('|').unwrap_or(s);
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b' ' {
        end -= 1;
    }
    if end > 0 && bytes[end - 1] == b'|' {
        &s[..end - 1]
    } else {
        s
    }
}

pub fn parse_table_align(cell: &str) -> crate::tokens::Align {
    let t = cell.trim_matches(' ');
    let bytes = t.as_bytes();
    if bytes.is_empty() {
        return crate::tokens::Align::None;
    }
    let left = bytes[0] == b':';
    let right = bytes[bytes.len() - 1] == b':';
    let start = if left { 1 } else { 0 };
    let end = if right { bytes.len() - 1 } else { bytes.len() };
    if start >= end {
        return crate::tokens::Align::None;
    }
    let mut i = start;
    while i < end {
        if bytes[i] != b'-' {
            return crate::tokens::Align::None;
        }
        i += 1;
    }
    match (left, right) {
        (false, true) => crate::tokens::Align::Right,
        (true, true) => crate::tokens::Align::Center,
        (true, false) => crate::tokens::Align::Left,
        (false, false) => crate::tokens::Align::None,
    }
}

pub fn remove_code_indent(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut first = true;
    for line in raw.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        let lb = line.as_bytes();
        let sp = leading_spaces_up_to(lb, 0, 4);
        let strip = if sp < 4 && sp < lb.len() && lb[sp] == b'\t' {
            sp + 1
        } else {
            sp
        };
        out.push_str(&line[strip..]);
    }
    out
}

pub fn indent_code_compensation(raw: &str, text: &str) -> String {
    let rb = raw.as_bytes();
    let mut indent_len = 0usize;
    let mut indent_chars = 0usize;
    while indent_len < rb.len() {
        let sl = js_space_len_at(rb, indent_len);
        if sl > 0 {
            indent_len += sl;
            indent_chars += 1;
        } else {
            break;
        }
    }
    if indent_len == 0
        || indent_len + 3 > rb.len()
        || rb[indent_len] != b'`'
        || rb[indent_len + 1] != b'`'
        || rb[indent_len + 2] != b'`'
    {
        return String::from(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        let lb = line.as_bytes();
        let mut node_ws = 0usize;
        let mut strip_bytes = 0usize;
        let mut i = 0;
        while i < lb.len() && node_ws < indent_chars {
            let sl = js_space_len_at(lb, i);
            if sl > 0 {
                node_ws += 1;
                i += sl;
                strip_bytes = i;
            } else {
                break;
            }
        }
        out.push_str(&line[strip_bytes..]);
    }
    out
}

pub fn preprocess_pedantic_block(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut first = true;
    for line in src.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        let lb = line.as_bytes();
        let line_no_tab = if lb.contains(&b'\t') {
            let mut buf = String::with_capacity(line.len() + 8);
            let mut last = 0;
            let mut i = 0;
            while i < lb.len() {
                if lb[i] == b'\t' {
                    if i > last {
                        buf.push_str(&line[last..i]);
                    }
                    buf.push_str("    ");
                    last = i + 1;
                }
                i += 1;
            }
            if last < lb.len() {
                buf.push_str(&line[last..]);
            }
            buf
        } else {
            String::from(line)
        };
        let nb = line_no_tab.as_bytes();
        let mut all_spaces = !nb.is_empty();
        let mut j = 0;
        while j < nb.len() {
            if nb[j] != b' ' {
                all_spaces = false;
                break;
            }
            j += 1;
        }
        if !all_spaces {
            out.push_str(&line_no_tab);
        }
    }
    out
}

#[inline]
pub fn trim_end_raw(s: &str) -> &str {
    trim_end_js(s)
}

/// Joins lines with `'\n'` without pulling in `alloc::str::join_generic_copy` (~6.1 KB).
pub fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut cap = lines.len().saturating_sub(1);
    let mut i = 0;
    while i < lines.len() {
        cap += lines[i].len();
        i += 1;
    }
    let mut out = String::with_capacity(cap);
    let mut j = 0;
    while j < lines.len() {
        if j > 0 {
            out.push('\n');
        }
        out.push_str(&lines[j]);
        j += 1;
    }
    out
}
