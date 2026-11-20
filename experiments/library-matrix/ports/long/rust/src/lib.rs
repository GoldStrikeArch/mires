#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;

use alloc::string::{String, ToString};
use mires_js::{js_bindings, js_export, throw_value, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("long: Rust panic"))
}

js_bindings! {
    #[js(call = "Reflect.get")]
    fn property(v: JsValue, key: JsValue) -> JsValue;
    #[js(index)]
    fn at(v: JsValue, index: usize) -> JsValue;
    #[js(get = "length")]
    fn len(v: JsValue) -> usize;
}
fn field(v: JsValue, key: &str) -> JsValue {
    property(v, JsValue::erase(key))
}
fn words(v: JsValue) -> u64 {
    let lo = at(v, 0).read::<f64>() as i32 as u32;
    let hi = at(v, 1).read::<f64>() as i32 as u32;
    ((hi as u64) << 32) | (lo as u64)
}

fn radix(value: i32) -> Option<u64> {
    if value == 0 {
        Some(10)
    } else if (2..=36).contains(&value) {
        Some(value as u64)
    } else {
        None
    }
}

fn parse(mut text: &str, base: i32) -> Option<u64> {
    let mut negative = false;
    loop {
        if text.is_empty() {
            return None;
        }
        if matches!(text, "NaN" | "Infinity" | "+Infinity" | "-Infinity") {
            return Some(0);
        }
        radix(base)?;
        if let Some(rest) = text.strip_prefix('-') {
            text = rest;
            negative = !negative;
        } else {
            break;
        }
    }
    if text.as_bytes().contains(&b'-') {
        return None;
    }
    let r = radix(base)?;
    let mut value = 0u64;
    for group in text.as_bytes().chunks(8) {
        let mut i = 0;
        while i < group.len() && (group[i].is_ascii_whitespace() || group[i] == 11) {
            i += 1;
        }
        // WORKAROUND(MIRES-019): compare byte values, not a promoted Option<&u8>.
        if group.get(i).copied() == Some(b'+') {
            i += 1;
        }
        if r == 16
            && group.get(i).copied() == Some(b'0')
            && matches!(group.get(i + 1), Some(b'x' | b'X'))
        {
            i += 2;
        }
        let mut digits = 0u64;
        while i < group.len() {
            if let Some(d) = (group[i] as char).to_digit(r as u32) {
                digits = digits * r + d as u64;
            } else {
                break;
            }
            i += 1;
        }
        value = value
            .wrapping_mul(r.pow(group.len() as u32))
            .wrapping_add(digits);
    }
    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

fn format(value: u64, unsigned: bool, base: i32) -> Option<String> {
    let r = radix(base)?;
    let neg = !unsigned && (value as i64) < 0;
    let mut mag = if neg { value.wrapping_neg() } else { value };
    let mut backwards = [0u8; 64];
    let mut n = 0;
    loop {
        backwards[n] = b"0123456789abcdefghijklmnopqrstuvwxyz"[(mag % r) as usize];
        n += 1;
        mag /= r;
        if mag == 0 {
            break;
        }
    }
    let mut out = String::with_capacity(n + neg as usize);
    if neg {
        out.push('-');
    }
    for i in (0..n).rev() {
        out.push(backwards[i] as char);
    }
    Some(out)
}

fn divide(a: u64, b: u64, unsigned: bool) -> Option<u64> {
    if b == 0 {
        None
    } else if unsigned {
        Some(a / b)
    }
    // Version 5.3.2's WebAssembly fast path tests the dividend's high word only.
    else if (a >> 32) == 0x80000000 && b == u64::MAX {
        Some(a)
    } else {
        Some(((a as i64) / (b as i64)) as u64)
    }
}

fn compare(a: u64, b: u64, u: bool, v: bool) -> i32 {
    if a == b && (u == v || (a as i64) >= 0) {
        return 0;
    }
    let an = !u && (a as i64) < 0;
    let bn = !v && (b as i64) < 0;
    let less = if an != bn {
        an
    } else if u {
        a < b
    } else {
        (a.wrapping_sub(b) as i64) < 0
    };
    if less {
        -1
    } else {
        1
    }
}

fn run_command(c: JsValue) -> Option<String> {
    let a = words(field(c, "a"));
    let b = words(field(c, "b"));
    let u = field(c, "u").read::<bool>();
    let v = field(c, "v").read::<bool>();
    let count = field(c, "n").read::<f64>();
    let n = count as i32;
    let shift = ((count % 64.0) as i32) as u32;
    let r = field(c, "r").read::<f64>() as i32;
    let value = match field(c, "op").read::<&str>() {
        "fromBits" | "toString" => a,
        "fromString" => parse(field(c, "s").read::<&str>(), n)?,
        "add" => a.wrapping_add(b),
        "subtract" => a.wrapping_sub(b),
        "multiply" => a.wrapping_mul(b),
        "divide" => divide(a, b, u)?,
        "modulo" if b != 0 => {
            if u {
                a % b
            } else {
                (a as i64).wrapping_rem(b as i64) as u64
            }
        }
        "and" => a & b,
        "or" => a | b,
        "xor" => a ^ b,
        "shiftLeft" => a.wrapping_shl(shift),
        "shiftRight" => (a as i64).wrapping_shr(shift) as u64,
        "shiftRightUnsigned" => a.wrapping_shr(shift),
        "rotl" => a.rotate_left(shift),
        "clz" => return Some(a.leading_zeros().to_string()),
        "ctz" => return Some(a.trailing_zeros().to_string()),
        "compare" => return Some(compare(a, b, u, v).to_string()),
        "mix" => {
            let mut acc = a;
            let mut checksum = 0i32;
            for i in 0..n {
                acc = acc
                    .wrapping_mul(b | 1)
                    .wrapping_add((i + 1) as u64)
                    .rotate_left(11)
                    ^ b;
                acc = acc.wrapping_add(divide(acc, (3 + (i & 255)) as u64, u)?);
                if (i & 7) == 0 {
                    let s = format(acc, u, r)?;
                    checksum = checksum
                        .wrapping_mul(33)
                        .wrapping_add(s.len() as i32 + s.as_bytes()[0] as i32);
                }
            }
            let mut out = format(acc, u, r)?;
            out.push(':');
            out.push_str(&checksum.to_string());
            return Some(out);
        }
        _ => return None,
    };
    format(value, u, r)
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let cases = field(input, "cases");
    let mut out = String::new();
    for i in 0..len(cases) {
        if i != 0 {
            out.push('\n');
        }
        match run_command(at(cases, i)) {
            Some(s) => out.push_str(&s),
            None => out.push('!'),
        }
    }
    JsValue::erase(out.as_str())
}
