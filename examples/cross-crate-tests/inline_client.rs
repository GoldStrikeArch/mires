#![no_std]
#![no_main]
#![deny(unsafe_code)]

use cross_inline_provider::{alternative, copied_pair, label, offset};
use mires_js::{js_export, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[js_export]
pub fn direct(value: u32) -> u32 {
    offset(value)
}

#[js_export]
pub fn pair(value: u32) -> u32 {
    copied_pair(value)
}

#[js_export]
pub fn text() -> &'static str {
    label()
}

#[js_export]
pub fn unequal(value: u32) -> bool {
    cross_inline_provider::unequal((value, 7))
}

#[js_export]
pub fn host_null(value: JsValue) -> bool {
    cross_inline_provider::host_null(value)
}

#[js_export]
pub fn host_call(function: JsValue, receiver: JsValue, argument: JsValue) -> JsValue {
    cross_inline_provider::host_call(function, receiver, argument)
}

#[js_export]
pub fn indirect(value: u32, other: bool) -> u32 {
    let function: fn(u32) -> u32 = if other { alternative } else { offset };
    function(value)
}
