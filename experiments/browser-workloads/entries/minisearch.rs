#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate browser_workloads;

use mires_js::{js_export, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }

#[js_export]
pub fn workload_run(options: JsValue) -> JsValue {
    let documents = browser_workloads::host::get(options, "documents");
    let query = browser_workloads::host::get(options, "query");
    let result = browser_workloads::minisearch::search_packed(documents.read(), query.read());
    JsValue::erase(result.as_str())
}
