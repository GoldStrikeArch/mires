//! JavaScript strings as UTF-16 code units, and the two text functions MiniSearch's defaults are:
//! `tokenize` (`text.split(/[\n\r\p{Z}\p{P}]+/u)`) and `processTerm` (`term.toLowerCase()`).
//!
//! Every string the index sees is a `Vec<u16>` / `&[u16]` of code units, because MiniSearch's
//! radix tree splits edges, measures lengths and computes edit distances on code units, and a
//! JavaScript string may hold lone surrogates (CONTRACT.md, "Strings are UTF-16").

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::separators::is_separator;

/// A string literal as UTF-16, for message templates and option defaults.
pub fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// The code point starting at `index` and its width in code units. A surrogate that is not half
/// of a well-formed pair is a code point of its own, as it is to a `u`-flag `RegExp`.
///
/// Comparisons rather than `Range::contains` in the hot text paths: under mires each `contains`
/// call builds a range object (NOTES.md, "Performance").
fn code_point_at(text: &[u16], index: usize) -> (u32, usize) {
    let unit = text[index];
    if unit >= 0xd800 && unit < 0xdc00 {
        if let Some(&next) = text.get(index + 1) {
            if next >= 0xdc00 && next < 0xe000 {
                return (0x10000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(next) - 0xdc00), 2);
            }
        }
    }
    (u32::from(unit), 1)
}

/// MiniSearch's default tokenizer, `text.split(/[\n\r\p{Z}\p{P}]+/u)`: the text between maximal
/// runs of separators. Like `split`, an empty text is one empty token, and a separator at either
/// end leaves an empty token there.
pub fn tokenize(text: &[u16]) -> Vec<&[u16]> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < text.len() {
        let (point, width) = code_point_at(text, index);
        if !is_separator(point) {
            index += width;
            continue;
        }
        tokens.push(&text[start..index]);
        index += width;
        while index < text.len() {
            let (point, width) = code_point_at(text, index);
            if !is_separator(point) {
                break;
            }
            index += width;
        }
        start = index;
    }
    tokens.push(&text[start..]);
    tokens
}

/// `String.prototype.toLowerCase`: Unicode default lower-casing with full mappings (`İ` becomes
/// `i` + U+0307) and the Final_Sigma context, which `str::to_lowercase` implements. A lone
/// surrogate is neither cased nor case-ignorable, so it ends a sigma's context exactly as the end
/// of a string does: each well-formed run is lower-cased on its own and lone surrogates are kept.
pub fn to_lower_case(term: &[u16]) -> Vec<u16> {
    let mut lower = Vec::with_capacity(term.len());
    lower_case_into(&mut lower, term);
    lower
}

/// [`to_lower_case`] into `lower`, replacing what it held (so one buffer serves many tokens).
pub fn lower_case_into(lower: &mut Vec<u16>, term: &[u16]) {
    lower.clear();
    if term.iter().all(|&unit| unit < 0x80) {
        lower.extend(term.iter().map(|&unit| if unit >= 0x41 && unit <= 0x5a { unit + 0x20 } else { unit }));
        return;
    }
    let mut run = String::new();
    for decoded in char::decode_utf16(term.iter().copied()) {
        match decoded {
            Ok(character) => run.push(character),
            Err(lone) => {
                lower.extend(run.to_lowercase().encode_utf16());
                run.clear();
                lower.push(lone.unpaired_surrogate());
            }
        }
    }
    lower.extend(run.to_lowercase().encode_utf16());
}

/// The value of a key that is an array index: the canonical decimal spelling of an integer below
/// 2^32 - 1. Such keys come first in `Object.keys`, in numeric order, before every other key.
pub fn array_index(key: &[u16]) -> Option<u32> {
    if key.is_empty() || key.len() > 10 || (key[0] == 0x30 && key.len() > 1) {
        return None;
    }
    let mut value = 0u32;
    for &unit in key {
        if unit < 0x30 || unit > 0x39 {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(unit - 0x30))?;
    }
    (value != u32::MAX).then_some(value)
}

/// The order `Object.keys` lists the keys of a plain object: array indices first, ascending, then
/// every other key in insertion order. As a comparison for a stable sort of the keys in insertion
/// order (two array indices are never equal: keys are distinct).
pub fn property_order(a: &[u16], b: &[u16]) -> Ordering {
    match (array_index(a), array_index(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Appends the decimal digits of an integer, as `Number.prototype.toString` writes it (`-0` is
/// `0`). Exact for every integer of magnitude below 2^53.
pub fn push_integer(out: &mut Vec<u16>, value: f64) {
    let mut magnitude = if value < 0.0 { -value } else { value };
    if value < 0.0 {
        out.push(0x2d);
    }
    let start = out.len();
    loop {
        let digit = magnitude % 10.0;
        out.push(0x30 + digit as u16);
        magnitude = (magnitude - digit) / 10.0;
        if magnitude == 0.0 {
            break;
        }
    }
    out[start..].reverse();
}

/// Joins strings with a separator, as `Array.prototype.join` does.
pub fn join<T: AsRef<[u16]>>(parts: &[T], separator: u16) -> Vec<u16> {
    let mut joined = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            joined.push(separator);
        }
        joined.extend_from_slice(part.as_ref());
    }
    joined
}
