#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::{js_export, JsValue};
#[js_export]
pub fn start(case: u32, token: JsValue, native: bool) -> JsValue {
    let future = cleanup_provider::work(case, token);
    if native {
        mires_js::future_to_native_promise(future)
    } else {
        mires_js::future_to_promise(future)
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
