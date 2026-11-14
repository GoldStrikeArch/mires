//! 293 with `middle` marked `#[inline(always)]`.
//!
//! Without MIR inlining this is 293's direct call. With `JSC_RUST_MIR_INLINE=on` rustc inlines
//! `middle` into `guarded_inlined`: the call disappears, and `middle`'s `UnwindTerminate(Abi)` edge
//! is left inside the caller, whose cleanup for `Guard` rustc had already removed. In unwind mode
//! the backend refuses both forms and names the inlined function and the fix. Under the default
//! `-Cpanic=abort` the program compiles, and `js-cleanup=on` keeps the drop on a host throw.
//! `scripts/unwind-mode-test.sh` runs both modes, with and without MIR inlining.

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

#[inline(always)]
extern "C" fn middle(throwing: bool) {
    unsafe { host_note(if throwing { -1.0 } else { 2.0 }) };
}

/// Notes `2` (or `-1`, which the host throws on), then drops the guard, which notes `1`.
#[unsafe(no_mangle)]
pub fn guarded_inlined(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    middle(throwing);
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
