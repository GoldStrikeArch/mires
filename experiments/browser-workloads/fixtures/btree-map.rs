#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use mires_js::{JsValue, js_export};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

/// Exercises the ordinary map shape a search index would naturally use.
#[js_export]
pub fn workload_run() -> JsValue {
    let mut terms = BTreeMap::<String, Vec<u32>>::new();
    for index in 0..16 {
        terms.entry(index.to_string()).or_default().push(index);
    }
    JsValue::erase(terms.len())
}
