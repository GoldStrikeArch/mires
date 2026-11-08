#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate browser_workloads;

use mires_js::{js_export, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }

#[js_export]
pub fn workload_run(options: JsValue) -> JsValue {
    let text = browser_workloads::host::get(options, "text");
    let compressed = browser_workloads::fflate::gzip_store(text.read::<&str>().as_bytes());
    let output = browser_workloads::host::uint8_array(compressed.len());
    for (index, byte) in compressed.iter().enumerate() {
        browser_workloads::host::set_index(output, index, JsValue::erase(f64::from(*byte)));
    }
    output
}
