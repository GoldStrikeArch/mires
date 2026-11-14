//! A foreign import declared `extern "C"`, called while a destructor is owed.
//!
//! Under `-Cpanic=unwind` (`JSC_PANIC=unwind`) rustc removes the cleanup edge of a call whose ABI
//! cannot unwind, so a JavaScript exception thrown by `host_note` would leave `Guard` undropped.
//! The backend refuses the program instead, with a message that names the fix: declare the import
//! `extern "C-unwind"` (291 is that form). Under the default `-Cpanic=abort` the program compiles,
//! and `js-cleanup=on` keeps the drop on a host throw. `scripts/unwind-mode-test.sh` runs both.

#![no_std]
#![no_main]

unsafe extern "C" {
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
