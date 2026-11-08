//! A host value cannot be read through a Rust primitive with a different value contract.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{js_bindings, js_export, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

js_bindings! {
    #[js(call = "console.log")]
    fn log(value: &str);
}

#[js_export]
pub fn rust_entry() {
    log("before invalid host read");
    let _: u32 = JsValue::erase("not a number").read();
}
