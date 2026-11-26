//! Raw JavaScript text token/string scanning, shared JS escape/string decoding, and
//! whole-program exclusion collection.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{JsName, LinkItem};
use crate::jsast::{self, Expr, Stmt, Visit};

use super::tag_encode::collect_boundary_exclusions;
use super::LoweredRow;

pub(super) struct ProgramExclusions {
    pub(super) raw_tokens: BTreeSet<String>,
    pub(super) raw_strings: BTreeSet<String>,
    pub(super) boundary_fields: BTreeSet<String>,
    pub(super) boundary_variants: BTreeSet<String>,
}

pub(super) fn decode_js_escape(text: &str, i: &mut usize, out: &mut String) {
    let bytes = text.as_bytes();
    if *i >= bytes.len() {
        return;
    }
    match bytes[*i] {
        b'\\' => {
            out.push('\\');
            *i += 1;
        }
        b'"' => {
            out.push('"');
            *i += 1;
        }
        b'\'' => {
            out.push('\'');
            *i += 1;
        }
        b'`' => {
            out.push('`');
            *i += 1;
        }
        b'n' => {
            out.push('\n');
            *i += 1;
        }
        b'r' => {
            out.push('\r');
            *i += 1;
        }
        b't' => {
            out.push('\t');
            *i += 1;
        }
        b'b' => {
            out.push('\u{0008}');
            *i += 1;
        }
        b'f' => {
            out.push('\u{000C}');
            *i += 1;
        }
        b'v' => {
            out.push('\u{000B}');
            *i += 1;
        }
        b'0' if !bytes.get(*i + 1).is_some_and(|c| c.is_ascii_digit()) => {
            out.push('\0');
            *i += 1;
        }
        b'\r' => {
            *i += 1;
            if bytes.get(*i) == Some(&b'\n') {
                *i += 1;
            }
        }
        b'\n' => {
            *i += 1;
        }
        b'x' if *i + 3 <= bytes.len()
            && bytes[*i + 1].is_ascii_hexdigit()
            && bytes[*i + 2].is_ascii_hexdigit() =>
        {
            if let Ok(v) = u32::from_str_radix(&text[*i + 1..*i + 3], 16) {
                if let Some(ch) = char::from_u32(v) {
                    out.push(ch);
                }
                *i += 3;
            } else {
                out.push('x');
                *i += 1;
            }
        }
        b'u' => {
            if bytes.get(*i + 1) == Some(&b'{') {
                let mut end = *i + 2;
                while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
                if end > *i + 2 && bytes.get(end) == Some(&b'}') {
                    if let Ok(v) = u32::from_str_radix(&text[*i + 2..end], 16) {
                        if let Some(ch) = char::from_u32(v) {
                            out.push(ch);
                        }
                        *i = end + 1;
                        return;
                    }
                }
            } else if *i + 5 <= bytes.len()
                && bytes[*i + 1..*i + 5].iter().all(u8::is_ascii_hexdigit)
            {
                if let Ok(code) = u32::from_str_radix(&text[*i + 1..*i + 5], 16) {
                    *i += 5;
                    if (0xD800..=0xDBFF).contains(&code)
                        && *i + 6 <= bytes.len()
                        && bytes[*i] == b'\\'
                        && bytes[*i + 1] == b'u'
                        && bytes[*i + 2..*i + 6].iter().all(u8::is_ascii_hexdigit)
                    {
                        if let Ok(lo) = u32::from_str_radix(&text[*i + 2..*i + 6], 16) {
                            if (0xDC00..=0xDFFF).contains(&lo) {
                                let combined = 0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00);
                                if let Some(ch) = char::from_u32(combined) {
                                    out.push(ch);
                                }
                                *i += 6;
                                return;
                            }
                        }
                    }
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                    }
                    return;
                }
            }
            out.push('u');
            *i += 1;
        }
        _ => {
            if let Some(ch) = text[*i..].chars().next() {
                out.push(ch);
                *i += ch.len_utf8();
            } else {
                *i += 1;
            }
        }
    }
}

/// Decodes a single- or double-quoted JS string literal starting at `text[*i]`, advancing `*i`
/// past the closing quote (or to `text.len()` if unterminated).
/// Returns `Some((decoded, closed))` when `text[*i]` is `"` or `'`.
pub(super) fn decode_js_quoted_string(text: &str, i: &mut usize) -> Option<(String, bool)> {
    let bytes = text.as_bytes();
    let quote = *bytes.get(*i)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    *i += 1;
    let mut decoded = String::new();
    while *i < bytes.len() {
        let b = bytes[*i];
        if b == quote {
            *i += 1;
            return Some((decoded, true));
        } else if b == b'\\' {
            *i += 1;
            decode_js_escape(text, i, &mut decoded);
        } else if b < 0x80 {
            decoded.push(b as char);
            *i += 1;
        } else if let Some(ch) = text[*i..].chars().next() {
            decoded.push(ch);
            *i += ch.len_utf8();
        } else {
            *i += 1;
        }
    }
    Some((decoded, false))
}

fn collect_js_text_span(
    text: &str,
    i: &mut usize,
    stop_on_close_brace: bool,
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    let bytes = text.as_bytes();
    let mut brace_depth = 0usize;
    while *i < bytes.len() {
        let b = bytes[*i];
        if b == b'/' && bytes.get(*i + 1) == Some(&b'/') {
            *i += 2;
            while *i < bytes.len() && bytes[*i] != b'\n' && bytes[*i] != b'\r' {
                *i += 1;
            }
        } else if b == b'/' && bytes.get(*i + 1) == Some(&b'*') {
            *i += 2;
            while *i + 1 < bytes.len() && !(bytes[*i] == b'*' && bytes[*i + 1] == b'/') {
                *i += 1;
            }
            *i = (*i + 2).min(bytes.len());
        } else if stop_on_close_brace && b == b'{' {
            brace_depth += 1;
            *i += 1;
        } else if stop_on_close_brace && b == b'}' {
            if brace_depth == 0 {
                *i += 1;
                return;
            }
            brace_depth -= 1;
            *i += 1;
        } else if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = *i;
            *i += 1;
            while *i < bytes.len()
                && (bytes[*i].is_ascii_alphanumeric() || bytes[*i] == b'_' || bytes[*i] == b'$')
            {
                *i += 1;
            }
            tokens.insert(text[start..*i].to_owned());
        } else if b == b'"' || b == b'\'' {
            if let Some((decoded, _)) = decode_js_quoted_string(text, i) {
                strings.insert(decoded);
            }
        } else if b == b'`' {
            *i += 1;
            let mut decoded = String::new();
            let mut has_interp = false;
            while *i < bytes.len() && bytes[*i] != b'`' {
                if bytes[*i] == b'\\' {
                    *i += 1;
                    decode_js_escape(text, i, &mut decoded);
                } else if bytes[*i] == b'$' && bytes.get(*i + 1) == Some(&b'{') {
                    has_interp = true;
                    *i += 2;
                    collect_js_text_span(text, i, true, tokens, strings);
                } else if bytes[*i] < 0x80 {
                    decoded.push(bytes[*i] as char);
                    *i += 1;
                } else if let Some(ch) = text[*i..].chars().next() {
                    decoded.push(ch);
                    *i += ch.len_utf8();
                } else {
                    *i += 1;
                }
            }
            if !has_interp {
                strings.insert(decoded);
            }
            if *i < bytes.len() {
                *i += 1;
            }
        } else if b < 0x80 {
            *i += 1;
        } else if let Some(ch) = text[*i..].chars().next() {
            *i += ch.len_utf8();
        } else {
            *i += 1;
        }
    }
}

pub(super) fn collect_js_text_tokens_and_strings(
    text: &str,
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    let mut i = 0;
    collect_js_text_span(text, &mut i, false, tokens, strings);
}

fn collect_raw_from_stmt(
    stmt: &Stmt,
    tokens: &mut BTreeSet<String>,
    strings: &mut BTreeSet<String>,
) {
    struct V<'a> {
        tokens: &'a mut BTreeSet<String>,
        strings: &'a mut BTreeSet<String>,
    }
    impl Visit for V<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            if let Stmt::Raw(text) = stmt {
                collect_js_text_tokens_and_strings(text, self.tokens, self.strings);
                return;
            }
            jsast::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &Expr) {
            if let Expr::Raw(text) = expr {
                collect_js_text_tokens_and_strings(text, self.tokens, self.strings);
                return;
            }
            jsast::walk_expr(self, expr);
        }
    }
    V { tokens, strings }.visit_stmt(stmt);
}

pub(super) fn collect_program_exclusions(
    table: &BTreeMap<JsName, LinkItem>,
    lowered: &[LoweredRow],
    live: &BTreeSet<JsName>,
) -> ProgramExclusions {
    let lowered_ok: BTreeSet<&JsName> = lowered
        .iter()
        .filter_map(|(name, res, _, _)| res.is_ok().then_some(name))
        .collect();
    let lowered_dts: BTreeSet<&JsName> = lowered
        .iter()
        .filter_map(|(name, res, _, _)| {
            res.as_ref()
                .ok()
                .and_then(|item| item.dts.as_ref().map(|_| name))
        })
        .collect();

    let mut raw_tokens = BTreeSet::new();
    let mut raw_strings = BTreeSet::new();

    for item in table.values() {
        if live.contains(&item.name) && !lowered_ok.contains(&item.name) {
            collect_js_text_tokens_and_strings(&item.js, &mut raw_tokens, &mut raw_strings);
        }
    }

    for (_, res, _, _) in lowered {
        if let Ok(item) = res {
            collect_raw_from_stmt(&item.decl, &mut raw_tokens, &mut raw_strings);
        }
    }

    let (boundary_fields, boundary_variants) =
        collect_boundary_exclusions(table, &lowered_ok, &lowered_dts, live);

    ProgramExclusions {
        raw_tokens,
        raw_strings,
        boundary_fields,
        boundary_variants,
    }
}
