//! Byte buffers crossing the host boundary in bulk: `mires_js::JsUint8Array`.
//!
//! A Rust `Vec<u8>` is a JavaScript array of numbers (or a `{ buf, off, len }` view over one), not
//! a `Uint8Array`, so handing bytes to the host is a conversion. `JsUint8Array::from_slice` is one
//! host operation that copies the whole range into a fresh `Uint8Array`; `copy_to_slice` and
//! `to_vec` are the one operation back. `scripts/extern-check.mjs` drives every entry point below
//! and checks exact byte values, exact lengths, slices that do not start at their buffer's first
//! element, `i8` storage read as its bytes, and copy semantics in both directions: a write on
//! either side after a crossing is never visible on the other.
//!
//! The panic handler throws, so a length mismatch in `copy_to_slice` is observable from the check
//! as an exception rather than as a hang.
//!
//! The last entry points hand the operations what no sound program can: a host object retyped as
//! a byte slice by a declaration that lies, and storage that is not bytes viewed as bytes through
//! `unsafe`. The export must answer each with a registered compiler abort and never an ordinary
//! exception, because the compiler classifies it as unable to throw.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use mires_js::{invoke, js_bindings, js_export, throw_value, JsUint8Array, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    throw_value(JsValue::erase("rust panic"))
}

/// `length` bytes of a pattern that reaches every value 0..=255.
fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index * 7 + 3) as u8).collect()
}

/// The bytes of `pattern(length)`, exported.
#[js_export]
pub fn bytes_pattern(length: u32) -> JsValue {
    JsUint8Array::from_slice(&pattern(length as usize)).as_value()
}

/// The two ends of the byte range and their neighbours.
#[js_export]
pub fn bytes_extremes() -> JsValue {
    JsUint8Array::from_slice(&[0, 255, 1, 254, 127, 128]).as_value()
}

/// An empty literal: a slice over an empty array constant (`{ buf: [], off: 0, len: 0 }`).
#[js_export]
pub fn bytes_empty_literal() -> JsValue {
    JsUint8Array::from_slice(&[]).as_value()
}

/// An empty `Vec` that never allocated: its slice is over the dangling pointer.
#[js_export]
pub fn bytes_empty_vec() -> JsValue {
    let bytes: Vec<u8> = Vec::new();
    JsUint8Array::from_slice(&bytes).as_value()
}

/// `pattern(300)[start..end]`: a slice whose first element is not its buffer's.
#[js_export]
pub fn bytes_middle(start: u32, end: u32) -> JsValue {
    let bytes = pattern(300);
    JsUint8Array::from(&bytes[start as usize..end as usize]).as_value()
}

/// A `Vec` that grew past its capacity and was then truncated: only `[0, len)` crosses, never the
/// capacity behind it.
#[js_export]
pub fn bytes_push_truncate() -> JsValue {
    let mut bytes = Vec::with_capacity(2);
    for value in 1..=10u8 {
        bytes.push(value * 25);
    }
    bytes.truncate(5);
    bytes.push(255);
    JsUint8Array::from_slice(&bytes).as_value()
}

/// `i8` storage read through a byte slice: each element crosses as its byte.
#[js_export]
pub fn bytes_signed() -> JsValue {
    let signed: Vec<i8> = vec![-128, -1, 0, 1, 127];
    // SAFETY: `i8` and `u8` have the same size and alignment and every bit pattern is valid for
    // both; the slice borrows `signed` for no longer than this call.
    let bytes = unsafe { core::slice::from_raw_parts(signed.as_ptr().cast::<u8>(), signed.len()) };
    JsUint8Array::from_slice(bytes).as_value()
}

/// A static byte string and a `&str`'s UTF-8 bytes.
#[js_export]
pub fn bytes_static_and_str(which: u32) -> JsValue {
    let bytes: &[u8] = match which {
        0 => b"\x00\xffstatic",
        _ => "h\u{e9}llo \u{1f600}".as_bytes(),
    };
    JsUint8Array::from_slice(bytes).as_value()
}

/// Rust to JavaScript to Rust: `pattern(length)` exported, imported back, compared.
#[js_export]
pub fn bytes_round_trip(length: u32) -> bool {
    let bytes = pattern(length as usize);
    let view = JsUint8Array::from_slice(&bytes);
    let back: Vec<u8> = Vec::from(view);
    let mut middle = vec![0; bytes.len() / 2];
    if !middle.is_empty() {
        let quarter = bytes.len() / 4;
        JsUint8Array::from_slice(&bytes[quarter..quarter + middle.len()])
            .copy_to_slice(&mut middle);
    }
    back == bytes && middle[..] == bytes[bytes.len() / 4..bytes.len() / 4 + middle.len()]
}

/// JavaScript to Rust to JavaScript: a host `Uint8Array` copied into a `Vec` and exported again.
#[js_export]
pub fn bytes_echo(input: JsValue) -> JsValue {
    let view = input.read::<JsUint8Array>();
    JsUint8Array::from_slice(&view.to_vec()).as_value()
}

/// A host `Uint8Array` copied into the middle of a larger Rust buffer of `0xee` bytes, which is
/// exported whole so the check can see both the copied range and the untouched bytes around it.
#[js_export]
pub fn bytes_copy_into_middle(input: JsValue, offset: u32) -> JsValue {
    let view = input.read::<JsUint8Array>();
    let offset = offset as usize;
    let mut bytes = vec![0xee; view.len() + 2 * offset];
    view.copy_to_slice(&mut bytes[offset..offset + view.len()]);
    JsUint8Array::from_slice(&bytes).as_value()
}

/// `copy_to_slice` into a destination of the wrong length: a panic, before anything is written.
#[js_export]
pub fn bytes_copy_mismatch(input: JsValue) -> u32 {
    let mut bytes = [7u8; 3];
    input.read::<JsUint8Array>().copy_to_slice(&mut bytes);
    u32::from(bytes[0])
}

/// Copy semantics in both directions, with the host's writes made by `mutate(view)`.
///
/// Answers a bit set of the failures, zero when every check holds:
/// 1. the host writing the exported view changed the Rust buffer;
/// 2. Rust writing its buffer after the export changed the view;
/// 4. the host writing the view after an import changed the imported `Vec`;
/// 8. the view does not hold what the host wrote.
#[js_export]
pub fn bytes_copy_isolation(mutate: JsValue) -> u32 {
    let mut failures = 0;
    let mut bytes = vec![10u8, 20, 30, 40];
    let view = JsUint8Array::from_slice(&bytes);
    let _ = invoke(mutate, JsValue::undefined(), &[view.as_value()]);
    if bytes != [10, 20, 30, 40] {
        failures |= 1;
    }
    bytes[1] = 99;
    if view.get(1) != Some(201) {
        failures |= 2;
    }
    let imported = view.to_vec();
    let _ = invoke(mutate, JsValue::undefined(), &[view.as_value()]);
    if imported != [200, 201, 202, 203] {
        failures |= 4;
    }
    if view.get(0) != Some(144) || view.len() != 4 {
        failures |= 8;
    }
    failures
}

/// A view whose buffer the host detaches after the conversion reads as zero bytes.
///
/// Answers `1000 * len before + 100 * len after + to_vec().len()`, after a `copy_to_slice` into an
/// empty destination, which a detached view accepts.
#[js_export]
pub fn bytes_after_detach(input: JsValue, detach: JsValue) -> u32 {
    let view = input.read::<JsUint8Array>();
    let before = view.len() as u32;
    let _ = invoke(detach, JsValue::undefined(), &[view.as_value()]);
    view.copy_to_slice(&mut []);
    before * 1000 + view.len() as u32 * 100 + view.to_vec().len() as u32
}

js_bindings! {
    /// A declaration that lies: `Object(value)` is the host value itself, typed as a Rust byte
    /// slice, which is how a host object reaches `from_slice` without `unsafe` code.
    #[js(call = "Object")]
    fn host_bytes(value: JsValue) -> &'static [u8];

    /// The same lie about a `Uint8Array`: the value is never checked.
    #[js(call = "Object")]
    fn unchecked_view(value: JsValue) -> JsUint8Array;
}

/// `to_vec` of a view the lying declaration mistyped: a `TypeError`, with no host code run.
#[js_export]
pub fn bytes_unchecked_to_vec(value: JsValue) -> JsValue {
    JsUint8Array::from_slice(&unchecked_view(value).to_vec()).as_value()
}

/// `from_slice` over whatever host value the lying declaration retypes.
#[js_export]
pub fn bytes_from_host_record(value: JsValue) -> JsValue {
    JsUint8Array::from_slice(host_bytes(value)).as_value()
}

/// `u64` storage viewed as sixteen bytes: there are two elements, not sixteen bytes, behind it.
#[js_export]
pub fn bytes_from_words() -> JsValue {
    let words: Vec<u64> = vec![0x0102_0304_0506_0708, 9];
    // SAFETY: sixteen initialized bytes in Rust; this backend's value model holds two elements.
    let bytes = unsafe { core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), 16) };
    JsUint8Array::from_slice(bytes).as_value()
}

/// Two host values viewed as two bytes: the elements are objects, not numbers.
#[js_export]
pub fn bytes_from_values(value: JsValue) -> JsValue {
    let values = [value, value];
    // SAFETY: deliberately unsound reinterpretation, for the check only.
    let bytes = unsafe { core::slice::from_raw_parts(values.as_ptr().cast::<u8>(), 2) };
    JsUint8Array::from_slice(bytes).as_value()
}
