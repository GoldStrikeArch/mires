//! A host callback owns its capture environment and does not run captured Rust destructors.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{callback0, invoke, js_bindings, js_export, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

js_bindings! {
    #[js(call = "console.log")]
    fn log(label: &str, value: JsValue);
}

struct Capture;

impl Drop for Capture {
    fn drop(&mut self) {
        log("capture-drop", JsValue::erase(true));
    }
}

#[js_export]
pub fn rust_entry() {
    let capture = Capture;
    let callback = callback0(move || {
        let _ = &capture;
        log("callback-run", JsValue::erase(true));
        JsValue::erase("done")
    });
    let result = invoke(callback, JsValue::undefined(), &[]);
    log(
        "callback-return",
        JsValue::erase(result.strictly_equals(JsValue::erase("done"))),
    );
}
