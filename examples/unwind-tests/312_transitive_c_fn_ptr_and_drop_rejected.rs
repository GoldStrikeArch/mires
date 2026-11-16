//! Transitive hazards inside `extern "C"` callees: indirect `fn` pointer call and `Drop` glue.
//!
//! Neither `bridge_fn_ptr` (which calls through an `extern "C" fn` pointer) nor `bridge_drop`
//! (which holds no other droppable value while dropping `DropThrower`, whose `Drop` calls an
//! `extern "C"` import) has `UnwindTerminate(Abi)` in its own body. Under `-Cpanic=unwind`
//! (`JSC_PANIC=unwind`), the transitive check refuses both `guarded_c_fn_ptr` and `guarded_c_drop`.
//! Under `-Cpanic=abort`, `js-cleanup=on` runs `Guard::drop` on the throw in both exports.

#![no_std]
#![no_main]

use core::hint::black_box;

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

struct DropThrower(bool);

impl Drop for DropThrower {
    #[inline(never)]
    fn drop(&mut self) {
        unsafe { host_throw(self.0) };
    }
}

#[inline(never)]
extern "C" fn call_host_throw(throwing: bool) {
    unsafe { host_throw(throwing) };
}

#[inline(never)]
pub extern "C" fn bridge_fn_ptr(f: extern "C" fn(bool), throwing: bool) {
    f(throwing);
}

#[inline(never)]
pub extern "C" fn bridge_drop(throwing: bool) {
    let _thrower = DropThrower(throwing);
}

/// Calls `bridge_fn_ptr` across a live `Guard(1.0)`.
#[unsafe(no_mangle)]
pub fn guarded_c_fn_ptr(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    let f: extern "C" fn(bool) = black_box(call_host_throw);
    bridge_fn_ptr(f, throwing);
    3
}

/// Calls `bridge_drop` across a live `Guard(1.0)`.
#[unsafe(no_mangle)]
pub fn guarded_c_drop(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    bridge_drop(throwing);
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
