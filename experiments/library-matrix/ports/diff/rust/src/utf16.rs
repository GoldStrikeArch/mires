// Rust strings contain only Unicode scalars. Inside the text algorithms, quote
// an isolated UTF-16 unit with ESC and one private-use payload, and double
// literal ESC characters. Each quoted pair is a single diff token.
use alloc::string::{String, ToString};
use mires_js::{js_bindings, JsValue};

const ESC: char = '\u{e800}';
const PAYLOAD: u32 = 0xe000;
js_bindings! {
    #[js(method="isWellFormed")] fn well_formed(v:JsValue)->bool;
    #[js(method="includes")] fn includes(v:JsValue,s:JsValue)->bool;
    #[js(method="codePointAt")] fn code_point(v:JsValue,i:usize)->u32;
    #[js(call="String.fromCodePoint")] fn from_point(c:u32)->JsValue;
    #[js(method="push")] fn push(a:JsValue,v:JsValue);
    #[js(method="join")] fn join(a:JsValue,s:JsValue)->JsValue;
}

pub fn read(v: JsValue) -> String {
    if well_formed(v) && !includes(v, JsValue::erase("\u{e800}")) {
        return v.read::<&str>().to_string();
    }
    let mut s = String::new();
    let mut i = 0;
    while i < super::len(v) {
        let c = code_point(v, i);
        if (0xd800..=0xdfff).contains(&c) {
            s.push(ESC);
            s.push(char::from_u32(PAYLOAD + c - 0xd800).unwrap());
        } else {
            let ch = char::from_u32(c).unwrap();
            if ch == ESC {
                s.push(ESC);
            }
            s.push(ch);
        }
        i += if c > 0xffff { 2 } else { 1 };
    }
    s
}

fn decoded(s: &str) -> (u32, usize) {
    let mut chars = s.chars();
    let c = chars.next().unwrap();
    if c == ESC {
        let payload = chars.next().unwrap();
        let code = if payload == ESC {
            ESC as u32
        } else {
            0xd800 + payload as u32 - PAYLOAD
        };
        (code, c.len_utf8() + payload.len_utf8())
    } else {
        (c as u32, c.len_utf8())
    }
}

pub fn point(s: &str) -> (char, usize) {
    let (code, bytes) = decoded(s);
    (
        char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER),
        bytes,
    )
}

// Git quotes file names as TextEncoder's scalar UTF-8.
pub fn scalar_text(s: &str) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i < s.len() {
        let (c, width) = point(&s[i..]);
        result.push(c);
        i += width;
    }
    result
}

/// The number of UTF-16 code units, JavaScript's `length`.
pub fn units(s: &str) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < s.len() {
        let (code, width) = decoded(&s[i..]);
        n += if code > 0xffff { 2 } else { 1 };
        i += width;
    }
    n
}

fn lone(unit: u32) -> String {
    let mut s = String::new();
    s.push(ESC);
    s.push(char::from_u32(PAYLOAD + unit - 0xd800).unwrap());
    s
}

/// `s.substring(1)`: dropping the first unit of an astral character leaves
/// its low surrogate.
pub fn drop_first_unit(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let (code, width) = decoded(s);
    let mut out = if code > 0xffff {
        lone(0xdc00 + ((code - 0x10000) & 0x3ff))
    } else {
        String::new()
    };
    out.push_str(&s[width..]);
    out
}

/// `s.substring(0, s.length - 1)`: dropping the last unit of an astral
/// character leaves its high surrogate.
pub fn drop_last_unit(s: &str) -> String {
    let mut last = (0, 0);
    let mut i = 0;
    while i < s.len() {
        let (code, width) = decoded(&s[i..]);
        last = (i, code);
        i += width;
    }
    let (start, code) = last;
    let mut out = s[..start].to_string();
    if code > 0xffff {
        out.push_str(&lone(0xd800 + ((code - 0x10000) >> 10)))
    }
    out
}

pub fn write(s: &str) -> JsValue {
    if !s.contains(ESC) {
        return JsValue::erase(s);
    }
    let parts = super::array(0);
    let mut i = 0;
    while i < s.len() {
        let (c, width) = decoded(&s[i..]);
        push(parts, from_point(c));
        i += width;
    }
    join(parts, JsValue::erase(""))
}
