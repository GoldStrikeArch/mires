//! Audit F02 reproducer: an `extern "C"` Rust bridge calling an `extern "C"` foreign import.
//!
//! Neither `bridge` (no drops held) nor `bridge`'s call to `host_throw` (`"C"` ABI gives
//! `UnwindAction::Unreachable`, not `UnwindTerminate(Abi)`) triggers a shallow `UnwindTerminate(Abi)`
//! check on `bridge`. Under `-Cpanic=unwind` (`JSC_PANIC=unwind`), the transitive check walks
//! `bridge` -> `host_throw` and refuses `audit_entry`'s call to `bridge`, naming `extern "C-unwind"`.
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
pub extern "C" fn bridge(throwing: bool) {
    unsafe { host_throw(throwing) };
}

/// Calls `bridge(throwing)` while holding `Guard(1.0)`.
#[unsafe(no_mangle)]
pub fn audit_entry(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    bridge(throwing);
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
