//! Positive control: mutually recursive pure `extern "C"` functions called across a live guard.
//!
//! `even` and `odd` form a cycle in the transitive call graph, with no host imports, function
//! pointers, `dyn` dispatch, inline asm, or missing bodies. The cycle-safe transitive check accepts
//! `guarded_pure_cycle` in both `-Cpanic=unwind` and `-Cpanic=abort`, and a subsequent
//! `extern "C-unwind"` host throw runs `Guard::drop`, recording `[9, 1]`.

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

#[inline(never)]
pub extern "C" fn even(n: u32) -> bool {
    if n == 0 {
        true
    } else {
        odd(n - 1)
    }
}

#[inline(never)]
pub extern "C" fn odd(n: u32) -> bool {
    if n == 0 {
        false
    } else {
        even(n - 1)
    }
}

/// Calls the mutually recursive pure `extern "C"` cycle while holding `Guard(1.0)`, then calls
/// `host_note` (`9` on throw, `2` on normal) so `[9, 1]` verifies cleanup execution.
#[unsafe(no_mangle)]
pub fn guarded_pure_cycle(throwing: bool) -> u32 {
    let _guard = Guard(1.0);
    let ok = even(black_box(4));
    unsafe {
        host_note(if throwing {
            9.0
        } else if ok {
            2.0
        } else {
            0.0
        });
    }
    3
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
