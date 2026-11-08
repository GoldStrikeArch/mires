#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{js_bindings, js_export};

js_bindings! {
    #[js(call = "noopPadNote")]
    fn note(value: f64);
}

struct Guard(u32);

impl Drop for Guard {
    fn drop(&mut self) {
        note(f64::from(self.0));
    }
}

#[inline(never)]
fn consume(guard: Guard) {
    note(f64::from(guard.0 + 100));
}

/// The first call's landing pad only resumes. The second call's pad drops `_guard`.
#[js_export]
pub fn mixed() {
    note(10.0);
    let _guard = Guard(1);
    note(11.0);
}

/// Every landing pad only resumes: the one local is `Copy`.
#[js_export]
pub fn copy_only(value: u32) -> u32 {
    let next = value + 1;
    note(20.0);
    next
}

/// Drop elaboration removes the moved guard's drop from the second call's landing pad. MIR
/// inlining copies `inner` into rustc's optimized body for this function.
#[js_export]
pub fn moved_then_inner() {
    let guard = Guard(3);
    consume(guard);
    inner();
}

#[inline(always)]
fn inner() {
    let _guard = Guard(4);
    note(40.0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
