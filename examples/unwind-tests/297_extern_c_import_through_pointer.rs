//! 290's `extern "C"` import called only through a function pointer held in a `static`.
//!
//! No call names the import, so a check of direct calls never sees it, but the pointer's ABI is
//! the import's: under `-Cpanic=unwind` (`JSC_PANIC=unwind`) the call loses its cleanup edge and
//! the backend refuses it, naming an `extern "C-unwind"` pointer type as the fix. Under the default
//! `-Cpanic=abort` the program compiles and `js-cleanup=on` keeps the drop on a host throw. (The
//! pointer is printed as the bare name `host_note`, so the harness also defines that global.)
//! `scripts/unwind-mode-test.sh` runs both.

#![no_std]
#![no_main]

unsafe extern "C" {
    fn host_note(value: f64);
}

static NOTE: unsafe extern "C" fn(f64) = host_note;

struct Guard(f64);

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { NOTE(self.0) };
    }
}

/// Notes `2` (or `-1`, which the host throws on), then drops the guard, which notes `1`.
#[unsafe(no_mangle)]
pub fn guarded_pointer(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    unsafe { NOTE(if throwing { -1.0 } else { 2.0 }) };
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
