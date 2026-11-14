//! A Rust-defined `extern "C" fn` between a guard-holding Rust frame and a throwing host call.
//!
//! Under `-Cpanic=unwind` (`JSC_PANIC=unwind`) rustc gives the call to `middle` no cleanup edge (its
//! ABI cannot unwind), and `middle`'s own call to `host_note` ends in `UnwindTerminate(Abi)`, which
//! the backend lowers as a rethrow so that an export called from JavaScript keeps its cleanup
//! (292). A host exception would therefore leave `guarded` with `Guard` undropped. The backend
//! refuses the program instead, with a message that names the fix: declare `middle`
//! `extern "C-unwind"`. Under the default `-Cpanic=abort` the program compiles, and
//! `js-cleanup=on` keeps the drop on a host throw. `scripts/unwind-mode-test.sh` runs both.

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

#[inline(never)]
extern "C" fn middle(throwing: bool) {
    unsafe { host_note(if throwing { -1.0 } else { 2.0 }) };
}

/// Notes `2` (or `-1`, which the host throws on), then drops the guard, which notes `1`.
#[unsafe(no_mangle)]
pub fn guarded_middle(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    middle(throwing);
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
