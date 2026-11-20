//! Deterministic native matchers and delimiter maskers for all inline-level rules in
//! `marked@18.1.1` (`src/rules.ts`, `src/Lexer.ts`, `src/Tokenizer.ts`).
//!
//! All matchers operate directly on `&[u8]` byte slices with zero `&str` suffix reslicing,
//! avoiding O(N²) `$rt_bstr_bytes` materialization under `js-str=bytes` (`PERF-003`).

use alloc::string::String;
use alloc::vec::Vec;
use mires_js::collections::HashMap;

use crate::block_rules::match_block_label;
use crate::helpers::normalize_label;
use crate::tokens::{LinkDef, Mode};
use crate::unicode::{
    decode_utf8_at, is_all_js_space_bytes, is_js_space, is_open_quote, is_punct,
    is_punct_or_space, js_space_len_at,
};

#[inline]
fn is_ascii_escape_punct(b: u8) -> bool {
    matches!(
        b,
        b'!' | b'"'
            | b'#'
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
            | b'<'
            | b'='
            | b'>'
            | b'?'
            | b'@'
            | b'['
            | b'\\'
            | b']'
            | b'^'
            | b'_'
            | b'`'
            | b'{'
            | b'|'
            | b'}'
            | b'~'
    )
}

/// Matches `inline.escape` at `bytes[pos..]`.
#[inline]
pub fn match_escape(bytes: &[u8], pos: usize) -> Option<u8> {
    if pos + 1 < bytes.len() && bytes[pos] == b'\\' && is_ascii_escape_punct(bytes[pos + 1]) {
        Some(bytes[pos + 1])
    } else {
        None
    }
}

fn match_inline_html_attribute(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    while i < n {
        let sl = js_space_len_at(bytes, i);
        if sl > 0 {
            i += sl;
        } else {
            break;
        }
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
    while i < n {
        let sl = js_space_len_at(bytes, i);
        if sl > 0 {
            i += sl;
        } else {
            break;
        }
    }
    if i < n && bytes[i] == b'=' {
        i += 1;
        while i < n {
            let sl = js_space_len_at(bytes, i);
            if sl > 0 {
                i += sl;
            } else {
                break;
            }
        }
        if i < n && bytes[i] == b'"' {
            i += 1;
            while i < n && bytes[i] != b'"' {
                i += 1;
            }
            if i < n && bytes[i] == b'"' {
                return Some(i + 1);
            }
            return Some(save);
        }
        if i < n && bytes[i] == b'\'' {
            i += 1;
            while i < n && bytes[i] != b'\'' {
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

/// Matches `inline.tag` at `bytes[pos..]`. Returns end byte index.
pub fn match_tag(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    if pos + 3 > n || bytes[pos] != b'<' {
        return None;
    }

    // 1. Inline comment: `<!--(?:-?>|[\s\S]*?-->)`
    if pos + 4 <= n && &bytes[pos..pos + 4] == b"<!--" {
        if pos + 5 <= n && bytes[pos + 4] == b'>' {
            return Some(pos + 5);
        }
        if pos + 6 <= n && bytes[pos + 4] == b'-' && bytes[pos + 5] == b'>' {
            return Some(pos + 6);
        }
        let mut i = pos + 4;
        while i + 2 < n {
            if bytes[i] == b'-' && bytes[i + 1] == b'-' && bytes[i + 2] == b'>' {
                return Some(i + 3);
            }
            i += 1;
        }
        return None;
    }

    // 2. Closing tag: `</[a-zA-Z][a-zA-Z0-9-]*\s*>`
    if bytes[pos + 1] == b'/' && pos + 4 <= n && bytes[pos + 2].is_ascii_alphabetic() {
        let mut i = pos + 3;
        while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        while i < n {
            let sl = js_space_len_at(bytes, i);
            if sl > 0 {
                i += sl;
            } else {
                break;
            }
        }
        if i < n && bytes[i] == b'>' {
            return Some(i + 1);
        }
        return None;
    }

    // 3. Open tag: `<[a-zA-Z][a-zA-Z0-9-]*(?:attribute)*?\s*/?>`
    if bytes[pos + 1].is_ascii_alphabetic() {
        let mut cur = pos + 2;
        while cur < n && (bytes[cur].is_ascii_alphanumeric() || bytes[cur] == b'-') {
            cur += 1;
        }
        loop {
            let mut c = cur;
            while c < n {
                let sl = js_space_len_at(bytes, c);
                if sl > 0 {
                    c += sl;
                } else {
                    break;
                }
            }
            if c < n && bytes[c] == b'/' {
                c += 1;
            }
            if c < n && bytes[c] == b'>' {
                return Some(c + 1);
            }
            if let Some(next_cur) = match_inline_html_attribute(bytes, cur) {
                cur = next_cur;
            } else {
                break;
            }
        }
        return None;
    }

    // 4. Processing instruction: `<\?[\s\S]*?\?>`
    if bytes[pos + 1] == b'?' {
        let mut i = pos + 2;
        while i + 1 < n {
            if bytes[i] == b'?' && bytes[i + 1] == b'>' {
                return Some(i + 2);
            }
            i += 1;
        }
        return None;
    }

    // 5 & 6. Declaration or CDATA
    if bytes[pos + 1] == b'!' {
        if pos + 9 <= n && &bytes[pos..pos + 9] == b"<![CDATA[" {
            let mut i = pos + 9;
            while i + 2 < n {
                if bytes[i] == b']' && bytes[i + 1] == b']' && bytes[i + 2] == b'>' {
                    return Some(i + 3);
                }
                i += 1;
            }
            return None;
        }
        if bytes[pos + 2].is_ascii_alphabetic() {
            let mut i = pos + 3;
            while i < n && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let sl = js_space_len_at(bytes, i);
            if sl > 0 {
                let mut j = i + sl;
                while j < n {
                    if bytes[j] == b'>' {
                        return Some(j + 1);
                    }
                    j += 1;
                }
            }
        }
    }

    None
}

#[inline]
pub fn is_start_a_tag(b: &[u8]) -> bool {
    b.len() >= 3 && b[0] == b'<' && (b[1] == b'a' || b[1] == b'A') && b[2] == b' '
}

#[inline]
pub fn is_end_a_tag(b: &[u8]) -> bool {
    b.len() >= 4 && b[0] == b'<' && b[1] == b'/' && (b[2] == b'a' || b[2] == b'A') && b[3] == b'>'
}

fn match_raw_block_tag_name(bytes: &[u8], start: usize) -> Option<usize> {
    let rest = &bytes[start..];
    for name in [b"pre".as_slice(), b"code", b"kbd", b"script"] {
        if rest.len() >= name.len() {
            let mut ok = true;
            let mut i = 0;
            while i < name.len() {
                if rest[i].to_ascii_lowercase() != name[i] {
                    ok = false;
                    break;
                }
                i += 1;
            }
            if ok {
                return Some(start + name.len());
            }
        }
    }
    None
}

pub fn is_start_pre_script_tag(b: &[u8]) -> bool {
    if b.len() < 4 || b[0] != b'<' {
        return false;
    }
    let Some(after) = match_raw_block_tag_name(b, 1) else {
        return false;
    };
    after < b.len() && (b[after] == b'>' || js_space_len_at(b, after) > 0)
}

pub fn is_end_pre_script_tag(b: &[u8]) -> bool {
    if b.len() < 5 || b[0] != b'<' || b[1] != b'/' {
        return false;
    }
    let Some(after) = match_raw_block_tag_name(b, 2) else {
        return false;
    };
    after < b.len() && (b[after] == b'>' || js_space_len_at(b, after) > 0)
}

/// Matches `inline.code` at `bytes[pos..]`, returning `(end_pos, tick_len)`.
pub fn match_inline_code_at(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    let n = bytes.len();
    if pos >= n || bytes[pos] != b'`' {
        return None;
    }
    let mut k = 0;
    while pos + k < n && bytes[pos + k] == b'`' {
        k += 1;
    }
    let body_start = pos + k;
    if body_start >= n {
        return None;
    }
    let mut i = body_start + 1;
    while i < n {
        if bytes[i] == b'`' {
            let mut run = 0;
            while i + run < n && bytes[i + run] == b'`' {
                run += 1;
            }
            if run == k {
                return Some((i + run, k));
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// Matches `inline.autolink` at `bytes[pos..]`, returning `(end_pos, is_email)`.
pub fn match_autolink(bytes: &[u8], pos: usize) -> Option<(usize, bool)> {
    let n = bytes.len();
    if pos + 4 > n || bytes[pos] != b'<' {
        return None;
    }
    let mut gt = pos + 1;
    while gt < n && bytes[gt] != b'>' {
        if bytes[gt] == b'<' || bytes[gt] < 0x20 {
            return None;
        }
        gt += 1;
    }
    if gt >= n || bytes[gt] != b'>' {
        return None;
    }
    let ib = &bytes[pos + 1..gt];
    // 1. URI scheme
    if !ib.is_empty() && ib[0].is_ascii_alphabetic() {
        let mut j = 1;
        while j < ib.len()
            && j < 32
            && (ib[j].is_ascii_alphanumeric() || ib[j] == b'+' || ib[j] == b'.' || ib[j] == b'-')
        {
            j += 1;
        }
        if j >= 2 && j < ib.len() && ib[j] == b':' {
            let mut k = j + 1;
            let mut valid = true;
            while k < ib.len() {
                let b = ib[k];
                if b < 0x20 || b == b'<' || b == b'>' || js_space_len_at(ib, k) > 0 {
                    valid = false;
                    break;
                }
                let (_, clen) = decode_utf8_at(ib, k);
                k += clen;
            }
            if valid {
                return Some((gt + 1, false));
            }
        }
    }
    // 2. Email
    if let Some(at) = ib.iter().position(|&b| b == b'@') {
        if at == 0 {
            return None;
        }
        let mut local_ok = true;
        let mut k = 0;
        while k < at {
            let b = ib[k];
            if !(b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'.' | b'!'
                        | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'/'
                        | b'='
                        | b'?'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'{'
                        | b'|'
                        | b'}'
                        | b'~'
                        | b'-'
                ))
            {
                local_ok = false;
                break;
            }
            k += 1;
        }
        if local_ok && is_autolink_email_domain(&ib[at + 1..]) {
            return Some((gt + 1, true));
        }
    }
    None
}

fn is_autolink_email_domain(dom: &[u8]) -> bool {
    let mut parts = 0usize;
    let mut i = 0usize;
    let n = dom.len();
    while i <= n {
        let start = i;
        while i < n && dom[i] != b'.' {
            i += 1;
        }
        let seg = &dom[start..i];
        if seg.is_empty() || seg.len() > 63 {
            return false;
        }
        if !seg[0].is_ascii_alphanumeric() || !seg[seg.len() - 1].is_ascii_alphanumeric() {
            return false;
        }
        let mut k = 0;
        while k < seg.len() {
            if !(seg[k].is_ascii_alphanumeric() || seg[k] == b'-') {
                return false;
            }
            k += 1;
        }
        parts += 1;
        if i == n {
            break;
        }
        i += 1;
    }
    parts >= 2
}

fn skip_inner_brackets(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    if pos >= n || bytes[pos] != b'[' {
        return None;
    }
    let mut i = pos + 1;
    while i < n {
        let b = bytes[i];
        if b == b']' {
            return Some(i + 1);
        }
        if b == b'[' {
            return None;
        }
        if b == b'\\' {
            if i + 1 >= n {
                return None;
            }
            let (_, clen) = decode_utf8_at(bytes, i + 1);
            i += 1 + clen;
        } else {
            i += 1;
        }
    }
    None
}

fn skip_outer_brackets(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    if pos >= n || bytes[pos] != b'[' {
        return None;
    }
    let mut i = pos + 1;
    while i < n {
        let b = bytes[i];
        if b == b']' {
            return Some(i + 1);
        }
        if b == b'[' {
            i = skip_inner_brackets(bytes, i)?;
        } else if b == b'\\' {
            if i + 1 >= n {
                return None;
            }
            let (_, clen) = decode_utf8_at(bytes, i + 1);
            i += 1 + clen;
        } else {
            i += 1;
        }
    }
    None
}

/// Returns up to 2 candidate closing `]` indices for `_inlineLabel` without heap allocation.
fn find_inline_label_ends(
    bytes: &[u8],
    start: usize,
    max_specials: Option<usize>,
) -> ([usize; 2], usize) {
    let n = bytes.len();
    let mut out = [0usize; 2];
    let mut out_len = 0usize;
    let mut alt_end: Option<usize> = None;
    let mut i = start;
    let mut specials = 0usize;

    while i < n {
        let b = bytes[i];
        if b == b']' {
            out[out_len] = i;
            out_len += 1;
            break;
        }
        if b == b'[' {
            if let Some(m) = max_specials {
                if specials >= m {
                    break;
                }
            }
            let Some(next) = skip_outer_brackets(bytes, i) else {
                break;
            };
            i = next;
            specials += 1;
        } else if b == b'\\' {
            if let Some(m) = max_specials {
                if specials >= m {
                    break;
                }
            }
            if i + 1 >= n {
                break;
            }
            let (_, clen) = decode_utf8_at(bytes, i + 1);
            i += 1 + clen;
            specials += 1;
        } else if b == b'`' {
            if let Some(m) = max_specials {
                if specials >= m {
                    break;
                }
            }
            let mut run = 0;
            while i + run < n && bytes[i + run] == b'`' {
                run += 1;
            }
            let after_run = i + run;
            if run >= 2 && after_run < n && bytes[after_run] == b']' && alt_end.is_none() {
                alt_end = Some(after_run);
            }
            let mut j = after_run;
            while j < n && bytes[j] != b'`' {
                j += 1;
            }
            if j < n && bytes[j] == b'`' {
                while j < n && bytes[j] == b'`' {
                    j += 1;
                }
                i = j;
                specials += 1;
            } else {
                break;
            }
        } else {
            i += 1;
        }
    }
    if let Some(ae) = alt_end {
        if out_len == 0 || out[0] != ae {
            out[out_len] = ae;
            out_len += 1;
        }
    }
    (out, out_len)
}

/// Exact port of `isLabelEndInsideToken` (`Tokenizer.ts` lines 81-116).
pub fn is_label_end_inside_token(bytes: &[u8], label_start: usize, label_end: usize) -> bool {
    let lb = &bytes[label_start..label_end];
    if !lb.contains(&b'<') {
        return false;
    }
    let n = lb.len();
    let mut i = 0;
    while i < n {
        if lb[i] == b'\\' {
            if i + 1 < n {
                let (_, clen) = decode_utf8_at(lb, i + 1);
                i += 1 + clen;
            } else {
                i += 1;
            }
            continue;
        }
        if lb[i] == b'`' {
            if let Some((end, _)) = match_inline_code_at(lb, i) {
                i = end;
                continue;
            }
        }
        if lb[i] != b'<' {
            i += 1;
            continue;
        }
        let abs = label_start + i;
        let tok_end = match_tag(bytes, abs).or_else(|| match_autolink(bytes, abs).map(|(e, _)| e));
        if let Some(end) = tok_end {
            let len = end - abs;
            if len > n - i {
                return true;
            }
            i += len;
        } else {
            i += 1;
        }
    }
    false
}

fn match_inline_link_title(bytes: &[u8], pos: usize) -> Option<usize> {
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
            while i < n {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i < n && bytes[i] == b'\'' {
                        i += 1;
                    }
                } else if bytes[i] == b'\'' {
                    return Some(i + 1);
                } else {
                    i += 1;
                }
            }
            None
        }
        b'(' => {
            let mut i = pos + 1;
            while i < n {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i < n && bytes[i] == b')' {
                        i += 1;
                    }
                } else if bytes[i] == b')' {
                    return Some(i + 1);
                } else {
                    i += 1;
                }
            }
            None
        }
        _ => None,
    }
}

fn match_link_tail(bytes: &[u8], pos: usize) -> Option<(usize, Option<(usize, usize)>)> {
    let n = bytes.len();
    let mut i = pos;
    let mut has_space = false;
    while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
        has_space = true;
    }
    let mut has_nl = false;
    if i < n && bytes[i] == b'\n' {
        i += 1;
        has_nl = true;
        while i < n && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
    }
    if has_space || has_nl {
        if let Some(title_end) = match_inline_link_title(bytes, i) {
            let mut j = title_end;
            while j < n && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') {
                j += 1;
            }
            if j < n && bytes[j] == b')' {
                return Some((j + 1, Some((i, title_end))));
            }
        }
    }
    let mut j = pos;
    while j < n && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') {
        j += 1;
    }
    if j < n && bytes[j] == b')' {
        return Some((j + 1, None));
    }
    None
}

pub struct LinkSpan {
    pub raw_end: usize,
    pub is_image: bool,
    pub label_start: usize,
    pub label_end: usize,
    pub href_start: usize,
    pub href_end: usize,
    pub title: Option<(usize, usize)>,
}

/// Matches `inline.link` at `bytes[pos..]`.
pub fn match_inline_link(bytes: &[u8], pos: usize, pedantic: bool) -> Option<LinkSpan> {
    let n = bytes.len();
    let (is_image, open_bracket) = if pos + 1 < n && bytes[pos] == b'!' && bytes[pos + 1] == b'[' {
        (true, pos + 1)
    } else if pos < n && bytes[pos] == b'[' {
        (false, pos)
    } else {
        return None;
    };
    let label_start = open_bracket + 1;
    let (ends, ends_len) = find_inline_label_ends(bytes, label_start, None);
    let mut ei = 0;
    while ei < ends_len {
        let close_bracket = ends[ei];
        ei += 1;
        if close_bracket + 1 >= n || bytes[close_bracket + 1] != b'(' {
            continue;
        }
        let paren_open = close_bracket + 1;

        if pedantic {
            let mut j = paren_open + 1;
            while j < n && bytes[j] != b')' && bytes[j] != b'\n' {
                j += 1;
            }
            if j < n && bytes[j] == b')' {
                return Some(LinkSpan {
                    raw_end: j + 1,
                    is_image,
                    label_start,
                    label_end: close_bracket,
                    href_start: paren_open + 1,
                    href_end: j,
                    title: None,
                });
            }
            continue;
        }

        let mut i = paren_open + 1;
        while i < n && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n') {
            i += 1;
        }
        if i >= n {
            continue;
        }
        if bytes[i] == b')' {
            return Some(LinkSpan {
                raw_end: i + 1,
                is_image,
                label_start,
                label_end: close_bracket,
                href_start: i,
                href_end: i,
                title: None,
            });
        }
        if bytes[i] == b'<' {
            let mut j = i + 1;
            let mut ok = true;
            while j < n {
                let b = bytes[j];
                if b == b'>' {
                    break;
                }
                if b == b'\n' || b == b'<' {
                    ok = false;
                    break;
                }
                if b == b'\\' {
                    if j + 1 < n && bytes[j + 1] != b'\n' {
                        let (_, clen) = decode_utf8_at(bytes, j + 1);
                        j += 1 + clen;
                    } else {
                        ok = false;
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            if ok && j > i + 1 && j < n && bytes[j] == b'>' {
                if let Some((raw_end, title)) = match_link_tail(bytes, j + 1) {
                    return Some(LinkSpan {
                        raw_end,
                        is_image,
                        label_start,
                        label_end: close_bracket,
                        href_start: i,
                        href_end: j + 1,
                        title,
                    });
                }
            }
        }
        let href_start = i;
        let mut run_end = i;
        while run_end < n && bytes[run_end] > 0x20 {
            run_end += 1;
        }
        if run_end > href_start {
            if let Some((raw_end, title)) = match_link_tail(bytes, run_end) {
                return Some(LinkSpan {
                    raw_end,
                    is_image,
                    label_start,
                    label_end: close_bracket,
                    href_start,
                    href_end: run_end,
                    title,
                });
            }
            let mut cand = run_end;
            while cand > href_start {
                cand -= 1;
                if bytes[cand] == b')' && cand > href_start {
                    return Some(LinkSpan {
                        raw_end: cand + 1,
                        is_image,
                        label_start,
                        label_end: close_bracket,
                        href_start,
                        href_end: cand,
                        title: None,
                    });
                }
            }
        }
    }
    None
}

pub fn match_pedantic_href_title(href: &str) -> Option<(&str, &str)> {
    let bytes = href.as_bytes();
    let n = bytes.len();
    let mut q = 0;
    while q < n && bytes[q] != b'\'' && bytes[q] != b'"' {
        q += 1;
    }
    if q >= n {
        return None;
    }
    let quote_char = bytes[q];
    let mut close_q = n;
    let mut k = n;
    while k > q + 1 {
        k -= 1;
        if bytes[k] == quote_char {
            close_q = k;
            break;
        }
    }
    if close_q == n {
        return None;
    }
    let mut ws_start = q;
    while ws_start > 0 {
        let (ch, len) = crate::unicode::decode_utf8_rev(bytes, ws_start);
        if is_js_space(ch) {
            ws_start -= len;
        } else {
            break;
        }
    }
    if ws_start == q || ws_start == 0 {
        return None;
    }
    Some((&href[..ws_start], &href[q + 1..close_q]))
}

pub struct ReflinkSpan {
    pub raw_end: usize,
    pub is_image: bool,
    pub label_start: usize,
    pub label_end: usize,
    pub ref_span: Option<(usize, usize)>,
}

pub fn match_reflink_at(
    bytes: &[u8],
    pos: usize,
    pedantic: bool,
    bounded: bool,
) -> Option<ReflinkSpan> {
    let n = bytes.len();
    let (is_image, open_bracket) = if pos + 1 < n && bytes[pos] == b'!' && bytes[pos + 1] == b'[' {
        (true, pos + 1)
    } else if pos < n && bytes[pos] == b'[' {
        (false, pos)
    } else {
        return None;
    };
    let label_start = open_bracket + 1;
    let max_sp = if bounded { Some(999) } else { None };

    let (ends, ends_len) = find_inline_label_ends(bytes, label_start, max_sp);
    let mut ei = 0;
    while ei < ends_len {
        let close_bracket = ends[ei];
        ei += 1;
        let mut r_open = close_bracket + 1;
        if pedantic && !bounded {
            while r_open < n {
                let sl = js_space_len_at(bytes, r_open);
                if sl > 0 {
                    r_open += sl;
                } else {
                    break;
                }
            }
            if r_open < n && bytes[r_open] == b'[' {
                let mut r_close = r_open + 1;
                while r_close < n && bytes[r_close] != b']' {
                    r_close += 1;
                }
                if r_close < n && bytes[r_close] == b']' {
                    return Some(ReflinkSpan {
                        raw_end: r_close + 1,
                        is_image,
                        label_start,
                        label_end: close_bracket,
                        ref_span: if r_close > r_open + 1 {
                            Some((r_open + 1, r_close))
                        } else {
                            None
                        },
                    });
                }
            }
        } else if r_open < n && bytes[r_open] == b'[' {
            if let Some(r_close) = match_block_label(bytes, r_open + 1, max_sp) {
                return Some(ReflinkSpan {
                    raw_end: r_close + 1,
                    is_image,
                    label_start,
                    label_end: close_bracket,
                    ref_span: Some((r_open + 1, r_close)),
                });
            }
        }
    }

    if let Some(close) = match_block_label(bytes, label_start, max_sp) {
        let end = if close + 2 < n && bytes[close + 1] == b'[' && bytes[close + 2] == b']' {
            close + 3
        } else {
            close + 1
        };
        if bounded && end < n && bytes[end] == b'(' {
            return None;
        }
        return Some(ReflinkSpan {
            raw_end: end,
            is_image,
            label_start,
            label_end: close,
            ref_span: None,
        });
    }

    None
}

fn match_block_skip_at(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    if pos >= n {
        return None;
    }
    match bytes[pos] {
        b'[' => {
            let mut i = pos + 1;
            let mut close_bracket = None;
            while i < n {
                let b = bytes[i];
                if b == b']' {
                    close_bracket = Some(i);
                    break;
                }
                if b == b'[' {
                    return None;
                }
                if b == b'`' {
                    let (end, _) = match_block_skip_code_at(bytes, i)?;
                    i = end;
                } else {
                    i += 1;
                }
            }
            let cb = close_bracket?;
            if cb + 1 >= n || bytes[cb + 1] != b'(' {
                return None;
            }
            let mut j = cb + 2;
            while j < n {
                let b = bytes[j];
                if b == b')' {
                    return Some((j + 1) - pos);
                }
                if b == b'\\' {
                    if j + 1 >= n {
                        return None;
                    }
                    let (_, clen) = decode_utf8_at(bytes, j + 1);
                    j += 1 + clen;
                } else if b == b'(' {
                    j += 1;
                    let mut closed = false;
                    while j < n {
                        let b2 = bytes[j];
                        if b2 == b')' {
                            j += 1;
                            closed = true;
                            break;
                        }
                        if b2 == b'(' {
                            return None;
                        }
                        if b2 == b'\\' {
                            if j + 1 >= n {
                                return None;
                            }
                            let (_, clen) = decode_utf8_at(bytes, j + 1);
                            j += 1 + clen;
                        } else {
                            j += 1;
                        }
                    }
                    if !closed {
                        return None;
                    }
                } else {
                    j += 1;
                }
            }
            None
        }
        b'`' => {
            if pos > 0 && bytes[pos - 1] == b'`' {
                return None;
            }
            match_block_skip_code_at(bytes, pos).map(|(end, _)| end - pos)
        }
        b'<' => {
            if pos + 1 < n && bytes[pos + 1] == b' ' {
                return None;
            }
            let mut j = pos + 1;
            while j < n {
                if bytes[j] == b'>' {
                    return Some((j + 1) - pos);
                }
                if bytes[j] == b'<' {
                    return None;
                }
                j += 1;
            }
            None
        }
        _ => None,
    }
}

fn match_block_skip_code_at(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    let n = bytes.len();
    let mut k = 0;
    while pos + k < n && bytes[pos + k] == b'`' {
        k += 1;
    }
    if k == 0 {
        return None;
    }
    let body_start = pos + k;
    if body_start >= n || bytes[body_start] == b'`' {
        return None;
    }
    let mut i = body_start + 1;
    while i < n && bytes[i] != b'`' {
        i += 1;
    }
    if i >= n {
        return None;
    }
    let mut run = 0;
    while i + run < n && bytes[i + run] == b'`' {
        run += 1;
    }
    if run == k {
        Some((i + run, k))
    } else {
        None
    }
}

fn link_in_text(text: &str, bytes: &[u8], links: &HashMap<String, LinkDef>, pedantic: bool) -> bool {
    if !bytes.contains(&b'[') {
        return false;
    }
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if let Some(len) = match_block_skip_at(bytes, i) {
            if bytes[i] == b'['
                && !(i > 0 && bytes[i - 1] == b'!')
                && match_inline_link(&bytes[i..i + len], 0, pedantic).is_some()
            {
                return true;
            }
            i += len;
        } else {
            i += 1;
        }
    }
    let mut j = 0;
    while j < n {
        if bytes[j] == b'!' || bytes[j] == b'[' {
            if let Some(m) = match_reflink_at(bytes, j, false, true) {
                let slice_bytes = &bytes[j..m.raw_end];
                if !m.is_image {
                    if let Some(ref_start) = slice_bytes.iter().rposition(|&b| b == b'[') {
                        let key = normalize_label(&text[j + ref_start + 1..m.raw_end - 1]);
                        if links.contains_key(key.as_str()) {
                            let nested = ref_start > 1
                                && link_in_text(
                                    &text[j + 1..j + ref_start - 1],
                                    &bytes[j + 1..j + ref_start - 1],
                                    links,
                                    pedantic,
                                );
                            if !nested {
                                return true;
                            }
                        }
                    }
                }
                j = m.raw_end;
                continue;
            }
        }
        j += 1;
    }
    false
}

fn replace_reflink_search_bytes(
    src: &str,
    bytes: &[u8],
    links: &HashMap<String, LinkDef>,
    pedantic: bool,
    out: &mut Vec<u8>,
) {
    let n = bytes.len();
    let mut last = 0usize;
    let mut i = 0usize;
    while i < n {
        if bytes[i] == b'!' || bytes[i] == b'[' {
            if let Some(m) = match_reflink_at(bytes, i, false, true) {
                if i > last {
                    out.extend_from_slice(&bytes[last..i]);
                }
                let match_bytes = &bytes[i..m.raw_end];
                if let Some(ref_start) = match_bytes.iter().rposition(|&b| b == b'[') {
                    let key = normalize_label(&src[i + ref_start + 1..m.raw_end - 1]);
                    if links.contains_key(key.as_str()) {
                        if ref_start > 1 && !m.is_image {
                            let inner_str = &src[i + 1..i + ref_start - 1];
                            let inner_bytes = &bytes[i + 1..i + ref_start - 1];
                            if link_in_text(inner_str, inner_bytes, links, pedantic) {
                                out.push(b'[');
                                replace_reflink_search_bytes(
                                    inner_str,
                                    inner_bytes,
                                    links,
                                    pedantic,
                                    out,
                                );
                                out.push(b']');
                                out.push(b'[');
                                let ref_inner_len = match_bytes.len() - ref_start - 2;
                                let mut k = 0;
                                while k < ref_inner_len {
                                    out.push(b'a');
                                    k += 1;
                                }
                                out.push(b']');
                                i = m.raw_end;
                                last = i;
                                continue;
                            }
                        }
                        out.push(b'[');
                        let inner_len = match_bytes.len() - 2;
                        let mut k = 0;
                        while k < inner_len {
                            out.push(b'a');
                            k += 1;
                        }
                        out.push(b']');
                        i = m.raw_end;
                        last = i;
                        continue;
                    }
                }
                out.extend_from_slice(match_bytes);
                i = m.raw_end;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    if last < n {
        out.extend_from_slice(&bytes[last..]);
    }
}

/// Builds `maskedSrc` as a `Vec<u8>` (exact same UTF-8 byte length as `src`), operating directly
/// on byte buffers without intermediate JS string allocations.
pub fn build_masked_bytes(
    src: &str,
    bytes: &[u8],
    links: &HashMap<String, LinkDef>,
    pedantic: bool,
) -> Vec<u8> {
    let n = bytes.len();
    let mut buf: Vec<u8> = if !links.is_empty() && bytes.contains(&b'[') {
        let mut v = Vec::with_capacity(n);
        replace_reflink_search_bytes(src, bytes, links, pedantic, &mut v);
        v
    } else {
        bytes.to_vec()
    };

    // Pass 2: `anyPunctuationOutsideCode` (in-place mutation — preserves exact byte length!)
    if buf.contains(&b'`') || buf.contains(&b'\\') {
        let mut i = 0usize;
        while i < buf.len() {
            if buf[i] == b'`' {
                if let Some((end, k)) = match_inline_code_at(&buf, i) {
                    let mut j = i + k;
                    let mid_end = end - k;
                    while j < mid_end {
                        if buf[j] == b'`' {
                            buf[j] = b'+';
                        }
                        j += 1;
                    }
                    i = end;
                } else {
                    while i < buf.len() && buf[i] == b'`' {
                        i += 1;
                    }
                }
            } else if buf[i] == b'\\' && i + 1 < buf.len() {
                let (ch, clen) = decode_utf8_at(&buf, i + 1);
                if is_punct(ch) {
                    let mlen = 1 + clen;
                    let mut k = 0;
                    while k < mlen {
                        buf[i + k] = b'+';
                        k += 1;
                    }
                    i += mlen;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }

    // Pass 3: `blockSkip` (in-place mutation — preserves exact byte length!)
    if buf.contains(&b'[') || buf.contains(&b'`') || buf.contains(&b'<') {
        let mut i = 0usize;
        while i < buf.len() {
            let b = buf[i];
            if b == b'[' || b == b'`' || b == b'<' {
                if let Some(len) = match_block_skip_at(&buf, i) {
                    buf[i] = b'[';
                    let mut k = 1;
                    while k + 1 < len {
                        buf[i + k] = b'a';
                        k += 1;
                    }
                    buf[i + len - 1] = b']';
                    i += len;
                    continue;
                }
            }
            i += 1;
        }
    }

    buf
}

pub struct EmStrongLMatch {
    pub delim_char: u8,
    pub l_len: usize,
    pub next_is_punct: bool,
    pub is_und_non_punct: bool,
}

pub fn match_em_strong_l(bytes: &[u8], pos: usize, mode: Mode) -> Option<EmStrongLMatch> {
    let n = bytes.len();
    if pos >= n {
        return None;
    }
    let d = bytes[pos];
    if d != b'*' && d != b'_' {
        return None;
    }
    let mut l_len = 0;
    while pos + l_len < n && bytes[pos + l_len] == d {
        l_len += 1;
    }
    if pos + l_len >= n {
        return None;
    }
    let (next_ch, _) = decode_utf8_at(bytes, pos + l_len);
    let next_is_punct = if mode.is_pedantic() {
        !is_open_quote(next_ch) && is_punct(next_ch)
    } else if mode.is_gfm() {
        next_ch != '~' && is_punct(next_ch)
    } else {
        is_punct(next_ch)
    };
    if next_is_punct {
        return Some(EmStrongLMatch {
            delim_char: d,
            l_len,
            next_is_punct: true,
            is_und_non_punct: false,
        });
    }
    if !is_js_space(next_ch) {
        return Some(EmStrongLMatch {
            delim_char: d,
            l_len,
            next_is_punct: false,
            is_und_non_punct: d == b'_',
        });
    }
    None
}

#[derive(Clone, Copy)]
pub enum RDelimKind {
    RightOnly,
    LeftOnly,
    Both,
}

pub struct RDelimHit {
    pub delim_start: usize,
    pub delim_len: usize,
    pub kind: RDelimKind,
    pub next_pos: usize,
}

fn skip_orphan_inside_strong(bytes: &[u8], d: u8) -> usize {
    let n = bytes.len();
    let other = if d == b'*' { b'_' } else { b'*' };
    let mut i = 0;
    while i < n && bytes[i] != b'_' && bytes[i] != b'*' {
        i += 1;
    }
    if i + 1 >= n || bytes[i] != other || bytes[i + 1] != other {
        return 0;
    }
    i += 2;
    while i < n && bytes[i] != b'_' && bytes[i] != b'*' {
        i += 1;
    }
    if i >= n || bytes[i] != d {
        return 0;
    }
    i += 1;
    while i < n && bytes[i] != b'_' && bytes[i] != b'*' {
        i += 1;
    }
    if i + 1 < n && bytes[i] == other && bytes[i + 1] == other {
        i
    } else {
        0
    }
}

pub fn next_rdelim(
    bytes: &[u8],
    mut pos: usize,
    delim_byte: u8,
    mode: Mode,
    is_del: bool,
) -> Option<RDelimHit> {
    let n = bytes.len();
    if pos == 0 && !is_del {
        let skip = skip_orphan_inside_strong(bytes, delim_byte);
        if skip > 0 {
            pos = skip;
        }
    }

    let gfm_ast = !is_del && delim_byte == b'*' && mode.is_gfm();
    let pedantic = !is_del && mode.is_pedantic();
    let is_und = !is_del && delim_byte == b'_';

    let is_p = |ch: char| -> bool {
        if gfm_ast {
            ch != '~' && is_punct(ch)
        } else {
            is_punct(ch)
        }
    };
    let is_ps = |ch: char| -> bool {
        if gfm_ast {
            ch != '~' && is_punct_or_space(ch)
        } else {
            is_punct_or_space(ch)
        }
    };
    let is_nps = |ch: char| -> bool {
        if gfm_ast {
            ch == '~' || !is_punct_or_space(ch)
        } else {
            !is_punct_or_space(ch)
        }
    };

    while pos < n {
        if bytes[pos] != delim_byte {
            let mut next_d = pos;
            while next_d < n && bytes[next_d] != delim_byte {
                next_d += 1;
            }
            if next_d >= n {
                return None;
            }
            let mut prev_start = next_d - 1;
            while prev_start > pos && (bytes[prev_start] & 0xC0) == 0x80 {
                prev_start -= 1;
            }
            pos = prev_start;
        }

        let (prev_ch, prev_len) = decode_utf8_at(bytes, pos);
        let prev_end = pos + prev_len;
        if prev_end >= n || bytes[prev_end] != delim_byte {
            pos = prev_end;
            continue;
        }

        let mut run_end = prev_end;
        while run_end < n && bytes[run_end] == delim_byte {
            run_end += 1;
        }
        let run_len = run_end - prev_end;

        if is_del && run_len > 2 {
            pos = prev_end;
            continue;
        }
        if prev_ch as u32 == delim_byte as u32 {
            pos = prev_end;
            continue;
        }

        let next_ch = if run_end < n {
            Some(decode_utf8_at(bytes, run_end).0)
        } else {
            None
        };

        let kind = if is_p(prev_ch) && next_ch.map_or(true, is_js_space) {
            Some(RDelimKind::RightOnly)
        } else if is_nps(prev_ch) && next_ch.map_or(true, is_ps) {
            Some(RDelimKind::RightOnly)
        } else if (if pedantic { is_js_space(prev_ch) } else { is_ps(prev_ch) })
            && next_ch.is_some_and(is_nps)
        {
            Some(RDelimKind::LeftOnly)
        } else if is_js_space(prev_ch) && next_ch.is_some_and(is_p) {
            Some(RDelimKind::LeftOnly)
        } else if is_p(prev_ch) && next_ch.is_some_and(is_p) {
            Some(RDelimKind::Both)
        } else if next_ch.is_some_and(is_nps)
            && (if pedantic {
                is_p(prev_ch) || is_nps(prev_ch)
            } else if is_und {
                false
            } else {
                is_nps(prev_ch)
            })
        {
            Some(RDelimKind::Both)
        } else {
            None
        };

        if let Some(k) = kind {
            return Some(RDelimHit {
                delim_start: prev_end,
                delim_len: run_len,
                kind: k,
                next_pos: run_end,
            });
        }

        pos = prev_end;
    }

    None
}

pub fn match_del_l(bytes: &[u8], pos: usize) -> Option<(usize, bool)> {
    let n = bytes.len();
    if pos >= n || bytes[pos] != b'~' {
        return None;
    }
    let l_len = if pos + 1 < n && bytes[pos + 1] == b'~' {
        2
    } else {
        1
    };
    if pos + l_len >= n {
        return None;
    }
    let (next_ch, _) = decode_utf8_at(bytes, pos + l_len);
    if next_ch == '~' {
        return None;
    }
    if is_punct(next_ch) {
        Some((l_len, true))
    } else if !is_js_space(next_ch) {
        Some((l_len, false))
    } else {
        None
    }
}

pub fn match_br(bytes: &[u8], pos: usize, breaks: bool) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    if i < n && bytes[i] == b'\\' {
        i += 1;
    } else {
        while i < n && bytes[i] == b' ' {
            i += 1;
        }
        if !breaks && i - pos < 2 {
            return None;
        }
    }
    if i >= n || bytes[i] != b'\n' {
        return None;
    }
    let after_nl = i + 1;
    if is_all_js_space_bytes(bytes, after_nl, n) {
        return None;
    }
    let mut end = after_nl;
    while end < n && (bytes[end] == b' ' || bytes[end] == b'\t') {
        end += 1;
    }
    Some(end)
}

fn match_extended_email_at(bytes: &[u8], pos: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = pos;
    while i < n
        && (bytes[i].is_ascii_alphanumeric()
            || bytes[i] == b'.'
            || bytes[i] == b'_'
            || bytes[i] == b'+'
            || bytes[i] == b'-')
    {
        i += 1;
    }
    if i == pos || i >= n || bytes[i] != b'@' {
        return None;
    }
    i += 1;
    let d0 = i;
    while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
        i += 1;
    }
    if i == d0 {
        return None;
    }
    let mut valid_ends: Vec<usize> = Vec::new();
    let mut cur = i;
    while cur < n && bytes[cur] == b'.' {
        cur += 1;
        let mut last_alnum = None;
        while cur < n
            && (bytes[cur].is_ascii_alphanumeric() || bytes[cur] == b'-' || bytes[cur] == b'_')
        {
            if bytes[cur].is_ascii_alphanumeric() {
                last_alnum = Some(cur + 1);
            }
            cur += 1;
        }
        if let Some(la) = last_alnum {
            if la == cur {
                valid_ends.push(cur);
            } else {
                valid_ends.push(la);
                break;
            }
        } else {
            break;
        }
    }
    while let Some(end) = valid_ends.pop() {
        let next_blocked = end < n
            && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'-');
        if !next_blocked {
            return Some(end);
        }
    }
    None
}

#[inline]
fn match_protocol_prefix(bytes: &[u8], pos: usize) -> Option<usize> {
    let rest = &bytes[pos..];
    if rest.len() >= 8
        && rest[0].to_ascii_lowercase() == b'h'
        && rest[1].to_ascii_lowercase() == b't'
        && rest[2].to_ascii_lowercase() == b't'
        && rest[3].to_ascii_lowercase() == b'p'
        && rest[4].to_ascii_lowercase() == b's'
        && rest[5] == b':'
        && rest[6] == b'/'
        && rest[7] == b'/'
    {
        return Some(8);
    }
    if rest.len() >= 7
        && rest[0].to_ascii_lowercase() == b'h'
        && rest[1].to_ascii_lowercase() == b't'
        && rest[2].to_ascii_lowercase() == b't'
        && rest[3].to_ascii_lowercase() == b'p'
        && rest[4] == b':'
        && rest[5] == b'/'
        && rest[6] == b'/'
    {
        return Some(7);
    }
    if rest.len() >= 6
        && rest[0].to_ascii_lowercase() == b'f'
        && rest[1].to_ascii_lowercase() == b't'
        && rest[2].to_ascii_lowercase() == b'p'
        && rest[3] == b':'
        && rest[4] == b'/'
        && rest[5] == b'/'
    {
        return Some(6);
    }
    None
}

pub struct GfmUrlSpan {
    pub raw_end: usize,
    pub is_bare_email: bool,
    pub is_www: bool,
}

fn backpedal_once_len(bytes: &[u8]) -> usize {
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        if b == b'(' {
            let mut j = i + 1;
            while j < n && bytes[j] != b')' {
                j += 1;
            }
            if j < n && bytes[j] == b')' {
                i = j + 1;
            } else {
                break;
            }
        } else if b == b'&' {
            let mut j = i + 1;
            while j < n && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            if j > i + 1 && j + 1 == n && bytes[j] == b';' {
                break;
            }
            i += 1;
        } else if matches!(
            b,
            b'?' | b'!' | b'.' | b',' | b':' | b';' | b'*' | b'_' | b'\'' | b'"' | b'~' | b')'
        ) {
            let start = i;
            while i < n
                && matches!(
                    bytes[i],
                    b'?' | b'!'
                        | b'.'
                        | b','
                        | b':'
                        | b';'
                        | b'*'
                        | b'_'
                        | b'\''
                        | b'"'
                        | b'~'
                        | b')'
                )
            {
                i += 1;
            }
            if i == n {
                if i - start >= 2 {
                    i -= 1;
                } else {
                    i = start;
                }
                break;
            }
        } else {
            i += 1;
        }
    }
    i
}

fn backpedal_end(bytes: &[u8], pos: usize, mut end: usize) -> usize {
    loop {
        let next_len = backpedal_once_len(&bytes[pos..end]);
        if pos + next_len == end {
            break;
        }
        end = pos + next_len;
    }
    end
}

pub fn match_gfm_url(bytes: &[u8], pos: usize) -> Option<GfmUrlSpan> {
    let n = bytes.len();
    let rest = &bytes[pos..];

    if rest.starts_with(b"mailto:") {
        if let Some(end) = match_extended_email_at(bytes, pos + 7) {
            return Some(GfmUrlSpan {
                raw_end: backpedal_end(bytes, pos, end),
                is_bare_email: false,
                is_www: false,
            });
        }
    } else if rest.starts_with(b"xmpp:") {
        if let Some(mut end) = match_extended_email_at(bytes, pos + 5) {
            if end + 1 < n
                && bytes[end] == b'/'
                && (bytes[end + 1].is_ascii_alphanumeric()
                    || bytes[end + 1] == b'@'
                    || bytes[end + 1] == b'.')
            {
                end += 1;
                while end < n
                    && (bytes[end].is_ascii_alphanumeric()
                        || bytes[end] == b'@'
                        || bytes[end] == b'.')
                {
                    end += 1;
                }
            }
            return Some(GfmUrlSpan {
                raw_end: backpedal_end(bytes, pos, end),
                is_bare_email: false,
                is_www: false,
            });
        }
    }

    let (prefix_len, is_www) = if let Some(pl) = match_protocol_prefix(bytes, pos) {
        (pl, false)
    } else if rest.starts_with(b"www.") {
        (4, true)
    } else {
        (0, false)
    };
    if prefix_len > 0 && pos + prefix_len < n {
        let b0 = bytes[pos + prefix_len];
        if b0.is_ascii_alphanumeric() || b0 == b'-' {
            let mut end = pos + prefix_len + 1;
            while end < n && bytes[end] != b'<' && js_space_len_at(bytes, end) == 0 {
                let (_, clen) = decode_utf8_at(bytes, end);
                end += clen;
            }
            return Some(GfmUrlSpan {
                raw_end: backpedal_end(bytes, pos, end),
                is_bare_email: false,
                is_www,
            });
        }
    }

    if let Some(end) = match_extended_email_at(bytes, pos) {
        return Some(GfmUrlSpan {
            raw_end: end,
            is_bare_email: true,
            is_www: false,
        });
    }

    None
}

#[inline]
fn is_br_spaces_nl(bytes: &[u8], pos: usize, breaks: bool) -> bool {
    let n = bytes.len();
    let mut i = pos;
    while i < n && bytes[i] == b' ' {
        i += 1;
    }
    i < n && bytes[i] == b'\n' && (breaks || i - pos >= 2)
}

#[inline]
fn is_gfm_text_email_char(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'.' | b'!'
                | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'/'
                | b'='
                | b'?'
                | b'_'
                | b'`'
                | b'{'
                | b'|'
                | b'}'
                | b'~'
                | b'-'
        )
}

#[inline]
fn starts_gfm_email_local(bytes: &[u8], pos: usize) -> bool {
    let n = bytes.len();
    let mut i = pos;
    while i < n && is_gfm_text_email_char(bytes[i]) {
        i += 1;
    }
    i > pos && i < n && bytes[i] == b'@'
}

/// Matches `inline.text` at `bytes[pos..]`, returning `end` index (> `pos`).
pub fn match_inline_text(bytes: &[u8], start: usize, mode: Mode) -> Option<usize> {
    let n = bytes.len();
    if start >= n {
        return None;
    }

    if !mode.is_gfm() {
        let mut pos = if bytes[start] == b'`' {
            let mut k = start;
            while k < n && bytes[k] == b'`' {
                k += 1;
            }
            k
        } else {
            start + decode_utf8_at(bytes, start).1
        };
        if is_br_spaces_nl(bytes, pos, false) {
            return Some(pos);
        }
        while pos < n {
            let b = bytes[pos];
            if matches!(b, b'\\' | b'<' | b'!' | b'[' | b'`' | b'*' | b'_') {
                return Some(pos);
            }
            let ch_len = if b < 0x80 {
                1
            } else {
                decode_utf8_at(bytes, pos).1
            };
            let next = pos + ch_len;
            if b != b' ' && is_br_spaces_nl(bytes, next, false) {
                return Some(next);
            }
            pos = next;
        }
        return Some(n);
    }

    let breaks = mode.is_breaks();
    if !bytes[start].is_ascii_alphanumeric() {
        let c0_end = start + decode_utf8_at(bytes, start).1;
        if bytes[c0_end..].starts_with(b"mailto:") || bytes[c0_end..].starts_with(b"xmpp:") {
            return Some(c0_end);
        }
    }

    let mut pos = if bytes[start] == b'`' {
        let mut k = start;
        while k < n && bytes[k] == b'`' {
            k += 1;
        }
        k
    } else if bytes[start] == b'~' {
        let mut k = start;
        while k < n && bytes[k] == b'~' {
            k += 1;
        }
        k
    } else {
        start + decode_utf8_at(bytes, start).1
    };

    if pos < n
        && (bytes[pos] == b'`'
            || bytes[pos] == b'~'
            || is_br_spaces_nl(bytes, pos, breaks)
            || starts_gfm_email_local(bytes, pos))
    {
        return Some(pos);
    }

    while pos < n {
        let b = bytes[pos];
        if matches!(b, b'\\' | b'<' | b'!' | b'[' | b'`' | b'*' | b'~' | b'_') {
            return Some(pos);
        }
        if breaks && is_br_spaces_nl(bytes, pos, true) {
            return Some(pos);
        }
        if (b == b'h' || b == b'H' || b == b'f' || b == b'F')
            && match_protocol_prefix(bytes, pos).is_some()
        {
            return Some(pos);
        }
        if b == b'w' && bytes[pos..].starts_with(b"www.") {
            return Some(pos);
        }

        let ch_len = if b < 0x80 {
            1
        } else {
            decode_utf8_at(bytes, pos).1
        };
        let next = pos + ch_len;

        if b != b' ' && is_br_spaces_nl(bytes, next, breaks) {
            return Some(next);
        }
        if !(b < 0x80 && b.is_ascii_alphanumeric())
            && (bytes[next..].starts_with(b"mailto:") || bytes[next..].starts_with(b"xmpp:"))
        {
            return Some(next);
        }
        if !(b < 0x80 && is_gfm_text_email_char(b)) && starts_gfm_email_local(bytes, next) {
            return Some(next);
        }

        pos = next;
    }

    Some(n)
}
