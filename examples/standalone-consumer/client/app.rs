//! No framework, DOM, proc macro, or hand-written JavaScript implementation.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::vec::Vec;

#[unsafe(no_mangle)]
pub fn sum_squares(n: u32) -> u32 {
    let values: Vec<u32> = (0..n).map(|n| n * n).collect();
    values.iter().copied().sum()
}

#[unsafe(no_mangle)]
pub fn greeting() -> &'static str { "Hello from Rust" }

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
