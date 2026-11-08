#![no_std]
#![no_main]
extern crate guardlib;
#[unsafe(no_mangle)]
pub fn rust_entry() {  }
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
