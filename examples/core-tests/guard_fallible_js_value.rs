//! A lookalike error type cannot receive a JavaScript exception through a fallible binding.

#![no_std]
#![no_main]

use mires_js::{js_bindings, js_export};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

mod lookalike {
    pub struct JsValue;
}

js_bindings! {
    #[js(get = "length", fallible)]
    fn bad(value: mires_js::JsValue) -> Result<usize, lookalike::JsValue>;
}

#[js_export]
pub fn rust_entry() {
    let _ = bad(mires_js::JsValue::undefined());
}
