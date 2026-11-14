//! 290 with the import declared `extern "cdecl"`: an ABI other than "C" that cannot unwind.
//!
//! The unwind mode's rule is rustc's own (`fn_can_unwind`), not a list of ABI names, so the call
//! loses its cleanup edge under `-Cpanic=unwind` (`JSC_PANIC=unwind`) exactly as 290's does, and
//! the backend refuses it the same way, naming the ABI and `extern "C-unwind"`. Under the default
//! `-Cpanic=abort` the program compiles, and `js-cleanup=on` keeps the drop on a host throw.
//! `scripts/unwind-mode-test.sh` runs both.

#![no_std]
#![no_main]
#![allow(unsupported_calling_conventions)]

unsafe extern "cdecl" {
    fn host_note(value: f64);
}

struct Guard(f64);

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(self.0) };
    }
}

/// Notes `2` (or `-1`, which the host throws on), then drops the guard, which notes `1`.
#[unsafe(no_mangle)]
pub fn guarded_import(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    unsafe { host_note(if throwing { -1.0 } else { 2.0 }) };
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
