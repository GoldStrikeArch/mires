//! 2-hop transitive `extern "C"` -> `extern "Rust"` -> `extern "C"` foreign import.
//!
//! `bridge` holds no drops and calls `rust_mid`, which calls the `"C"` import `host_throw`.
//! Under `-Cpanic=unwind` (`JSC_PANIC=unwind`), the transitive check walks `bridge` -> `rust_mid`
//! -> `host_throw` and refuses `guarded_c_rust_c`'s call to `bridge`, naming `extern "C-unwind"`.
//! Under `-Cpanic=abort`, `js-cleanup=on` runs `Guard::drop` on the throw.

#![no_std]
#![no_main]

unsafe extern "C-unwind" {
    fn host_note(value: f64);
}

unsafe extern "C" {
    fn host_throw(throwing: bool);
}

struct Guard(f64);

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(self.0) };
    }
}

#[inline(never)]
fn rust_mid(throwing: bool) {
    unsafe { host_throw(throwing) };
}

#[inline(never)]
pub extern "C" fn bridge(throwing: bool) {
    rust_mid(throwing);
}

/// Calls `bridge(throwing)` across a live `Guard(1.0)`.
#[unsafe(no_mangle)]
pub fn guarded_c_rust_c(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    bridge(throwing);
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
