//! A generic that keeps a destructor live across a call to its argument.
//!
//! `scripts/cleanup-transport-test.py` compiles `cleanup_metadata_middle.rs` against this crate's
//! `.rmeta` alone, the way cargo compiles the sysroot's `alloc` against `core`. The middle crate
//! instantiates `guarded` with its own closure, so its object holds the only copy of that
//! instance, and the copy has to be lowered from the cleanup MIR in this crate's rlib: rustc's
//! optimized MIR in the `.rmeta` lost the guard's cleanup path to `-Cpanic=abort`.
//!
//! `guarded::<fn() -> u32>` is instantiated in both the middle crate and the client, so two objects
//! carry a copy of one item, and the link keeps one of them for both calls.

#![no_std]

unsafe extern "C" {
    fn js_log_f64(value: f64);
}

pub struct Guard(pub u32);

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe { js_log_f64(f64::from(self.0)) }
    }
}

pub fn guarded<F: FnOnce() -> u32>(value: u32, f: F) -> u32 {
    let _guard = Guard(value);
    f() + value
}
