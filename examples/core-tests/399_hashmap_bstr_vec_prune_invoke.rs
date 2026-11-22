//! `mires_js::collections::HashMap` + `&str` (`js-str=bytes` & utf16) + `Vec<u32>` inside a
//! function holding a live `Drop` guard must prune ALL unnecessary `try/catch` blocks under
//! `RCGJS_WITNESS_CHECK=1` and preserve exact runtime semantics.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;

use mires_js::abort;
use mires_js::collections::HashMap;

unsafe extern "C-unwind" {
    fn js_log_i32(x: i32);
    fn js_log_str(s: &str);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    abort("panic")
}

struct LiveGuard<'a> {
    counter: &'a Cell<u32>,
}

impl Drop for LiveGuard<'_> {
    fn drop(&mut self) {
        self.counter.set(self.counter.get() + 1);
    }
}

#[inline(never)]
fn work(seed: u32, drops: &Cell<u32>) -> u32 {
    let _guard = LiveGuard { counter: drops };

    let mut map: HashMap<String, Vec<u32>> = HashMap::new();
    map.insert(String::from("alpha"), vec![seed, seed + 1]);
    map.insert(String::from("beta"), vec![seed + 2, seed + 3]);

    if let Some(v) = map.get_mut("alpha") {
        v.push(seed + 4);
    }

    let text = "  hello world  ";
    let trimmed = text.trim();
    let sub = &trimmed[0..5];
    let byte_sum: u32 = sub.as_bytes().iter().map(|&b| b as u32).sum();
    let ascii_bonus: u32 = if sub.is_ascii() { 1 } else { 0 };
    let roundtrip = core::str::from_utf8(sub.as_bytes()).unwrap();

    let mut acc = byte_sum + ascii_bonus + (roundtrip.len() as u32);
    if map.contains_key("beta") {
        acc += map.len() as u32;
    }
    for (k, v) in &map {
        acc += k.len() as u32;
        for &x in v {
            acc += x;
        }
    }

    if let Some(removed) = map.remove("beta") {
        acc += removed.len() as u32;
    }
    map.clear();
    acc += map.len() as u32;
    acc
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let drops = Cell::new(0);
    let total = work(10, &drops);
    unsafe {
        js_log_str("total");
        js_log_i32(total as i32);
        js_log_str("drops");
        js_log_i32(drops.get() as i32);
    }
}
