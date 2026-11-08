#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate browser_workloads;

use mires_js::{js_export, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }

#[js_export]
pub fn workload_run(options: JsValue) -> JsValue {
    browser_workloads::explorer::run(options)
}
