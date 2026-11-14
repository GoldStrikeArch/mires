//! An exported `extern "C" fn` that holds a guard across a host call.
//!
//! Under `-Cpanic=unwind` rustc ends the cleanup path of a function that cannot unwind in
//! `UnwindTerminate(Abi)` rather than `UnwindResume`. In this compiler what reaches it is a host
//! exception (a Rust panic is an abort every handler rethrows), and it leaves the export as the same
//! value once the guard has dropped: the link step lowers the terminator as the rethrow
//! `UnwindResume` is. In the default abort mode `js-cleanup=on` gives the same behaviour.
//! `scripts/unwind-mode-test.sh` runs both modes, normally and with the host throwing.

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

/// Notes `2` (or `-1`, which the host throws on), then drops the guard, which notes `7`.
#[unsafe(no_mangle)]
pub extern "C" fn exported_guard(throwing: bool) -> u32 {
    let _guard = Guard(7.0);
    unsafe { host_note(if throwing { -1.0 } else { 2.0 }) };
    5
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
