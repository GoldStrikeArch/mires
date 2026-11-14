//! The accepted form of 290: the same import declared `extern "C-unwind"`.
//!
//! The call keeps its cleanup edge under `-Cpanic=unwind`, so a JavaScript exception thrown by
//! `host_note` drops `Guard` on its way out and leaves `guarded_import` as the same value. In the
//! default abort mode `js-cleanup=on` gives the same behaviour. `scripts/unwind-mode-test.sh` runs
//! both modes, normally and with the host throwing.

#![no_std]
#![no_main]

unsafe extern "C-unwind" {
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
