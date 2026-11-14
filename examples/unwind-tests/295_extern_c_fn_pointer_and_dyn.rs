//! 293's `extern "C" fn` reached without naming it: through an `extern "C" fn` pointer, and as a
//! `dyn` method declared `extern "C"`.
//!
//! Neither callee is known where the call is compiled, and both ABIs cannot unwind, so under
//! `-Cpanic=unwind` (`JSC_PANIC=unwind`) both calls lose their cleanup edge. The backend refuses
//! each call and names the fix: an `extern "C-unwind"` pointer type, and a method declared
//! `extern "C-unwind"`. Under the default `-Cpanic=abort` the program compiles, and
//! `js-cleanup=on` keeps both drops on a host throw. `scripts/unwind-mode-test.sh` runs both.

#![no_std]
#![no_main]

use core::hint::black_box;

unsafe extern "C-unwind" {
    fn host_note(value: f64);
}

struct Guard(f64);

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { host_note(self.0) };
    }
}

extern "C" fn middle(throwing: bool) {
    unsafe { host_note(if throwing { -1.0 } else { 2.0 }) };
}

trait Act {
    extern "C" fn act(&self, throwing: bool);
}

struct Actor;

impl Act for Actor {
    extern "C" fn act(&self, throwing: bool) {
        unsafe { host_note(if throwing { -1.0 } else { 2.0 }) };
    }
}

/// Notes `2` (or `-1`, which the host throws on) through a pointer, then the guard notes `1`.
#[unsafe(no_mangle)]
pub fn guarded_pointer(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    let f: extern "C" fn(bool) = black_box(middle);
    f(throwing);
    3
}

/// The same through a `dyn Act`.
#[unsafe(no_mangle)]
pub fn guarded_dyn(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    let actor: &dyn Act = black_box(&Actor);
    actor.act(throwing);
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
