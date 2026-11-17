//! The compiled half of the `js-str` differential probes (`scripts/bstr-probes.sh`).
//!
//! `rust_entry` runs the in-Rust probes of `body.rs` and prints each line through the host's
//! `js_log_str`. The exports are the host-boundary probes: a host string in through an export's
//! `&str` parameter (the byte-string facade), through `JsValue::read` and out through
//! `JsValue::erase` (the host markers), and in and out of a `js_bindings!` call (`#[js_extern]`).

#![no_std]
#![no_main]

extern crate alloc;

mod body;
#[path = "prelude.rs"]
mod prelude;

use mires_js::{js_bindings, JsValue};
use prelude::*;

js_bindings! {
    #[js(call = "globalThis.bstrHostText")]
    fn host_text() -> &'static str;

    #[js(call = "globalThis.bstrHostSink")]
    fn host_sink(text: &str);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(location) = info.location() {
        let file = location.file();
        let base = match file.rfind('/') {
            Some(at) => &file[at + 1..],
            None => file,
        };
        print_str(&alloc::format!("panic {base}:{}", location.line()));
    }
    unsafe { js_abort("panic") }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    body::inside(&mut |line| print_str(line));
}

#[unsafe(no_mangle)]
pub fn bstr_len(s: &str) -> usize {
    s.len()
}

#[unsafe(no_mangle)]
pub fn bstr_chars(s: &str) -> usize {
    body::count_chars(s)
}

#[unsafe(no_mangle)]
pub fn bstr_echo(s: &str) -> &str {
    s
}

#[unsafe(no_mangle)]
pub fn bstr_tail(s: &str) -> &str {
    body::tail(s)
}

#[unsafe(no_mangle)]
pub fn bstr_cmp(a: &str, b: &str) -> i32 {
    body::compare(a, b)
}

#[unsafe(no_mangle)]
pub fn bstr_slice(s: &str, a: usize, b: usize) -> &str {
    body::slice(s, a, b)
}

#[unsafe(no_mangle)]
pub fn bstr_answer(value: JsValue, which: u32) -> JsValue {
    let text: &str = value.read();
    let answers = body::host_answers(text);
    JsValue::erase(answers[which as usize].1.as_str())
}

/// A host string through `JsValue::read::<&str>` and straight back out through `erase`.
#[unsafe(no_mangle)]
pub fn bstr_echo_value(value: JsValue) -> JsValue {
    let text: &str = value.read();
    JsValue::erase(text)
}

/// How many distinct `str` keys two host strings are, in a `mires_js` host-`Map` set.
#[unsafe(no_mangle)]
pub fn bstr_distinct(a: &str, b: &str) -> usize {
    let mut set = mires_js::collections::HashSet::<&str>::new();
    set.insert(a);
    set.insert(b);
    set.len()
}

#[unsafe(no_mangle)]
pub fn bstr_via_host() -> usize {
    let text = host_text();
    host_sink(&alloc::format!("{text}|{}", text.len()));
    text.len()
}
