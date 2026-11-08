#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate browser_workloads;

use mires_js::js_export;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

#[js_export]
pub fn workload_register() {
    browser_workloads::workbox_direct::register();
}
