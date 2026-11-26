//! Byte-string (`js-str=bytes`) const item synthesis and `Location` const rewriting.

use crate::item::{ItemKind, JsName, LinkItem};
use crate::jsast;

use super::build_synthesized_link_item;
use super::exclusions::decode_js_quoted_string;

/// Extracts the byte-string sibling `const bs$h... = "<byte_string>";` for a non-ASCII
/// backend-hoisted `const s$h... = "<utf16>";` constant under `js-str=bytes`.
pub(in crate::link) fn synthesize_bstr_const_item(s_name: &str, js: &str) -> Option<LinkItem> {
    let (_, rhs) = js.split_once('=')?;
    let rhs = rhs.trim();
    let mut i = 0;
    let (text, true) = decode_js_quoted_string(rhs, &mut i)? else {
        return None;
    };
    let tail = rhs[i..].trim_start();
    if !(tail.is_empty() || tail.starts_with(';')) {
        return None;
    }
    if text.is_ascii() || text.len() < crate::lower::js::constant::HOIST_STRING_BYTES {
        return None;
    }
    if crate::lower::js::constant::hoisted_name("s", &text) != s_name {
        return None;
    }
    let bs_name = JsName::new(crate::lower::js::constant::hoisted_name("bs", &text));
    let encoded = crate::lower::bstr::byte_string(&text);
    Some(build_synthesized_link_item(
        bs_name.clone(),
        ItemKind::Const,
        jsast::const_(bs_name.as_str(), jsast::string(encoded)),
        "hoisted byte-string constant",
    ))
}

/// Rewrites a producer-hoisted `const loc$h... = { filename: __rt.str_bytes("..."), ... };`
/// item into its `js-str=bytes` twin (`__rt.bstr_bytes("<byte_string>")`).
pub(in crate::link) fn rewrite_bstr_location_const_item(js: &str) -> Option<String> {
    const NEEDLE: &str = "__rt.str_bytes(";
    let call_start = js.find(NEEDLE)?;
    let rest = &js[call_start + NEEDLE.len()..];
    let mut i = 0;
    let (text, true) = decode_js_quoted_string(rest, &mut i)? else {
        return None;
    };
    let after_quote = &rest[i..];
    if !after_quote.starts_with(')') {
        return None;
    }
    let encoded = crate::lower::bstr::byte_string(&text);
    let encoded_lit = jsast::expr_to_string(&jsast::string(encoded));
    let mut out = String::with_capacity(js.len() + encoded_lit.len());
    out.push_str(&js[..call_start]);
    out.push_str("__rt.bstr_bytes(");
    out.push_str(&encoded_lit);
    out.push_str(after_quote);
    Some(out)
}
