//! Drop order and panic timing of `Vec` destruction, differentially vs the baseline (Phase 4).
//!
//! The element has its own destructor, which is a permanent exact decline
//! (`ElementHasOwnDrop`): this vector stays on the baseline carrier on every lane in every mode,
//! and its observable drop order pins what any native lowering would have to preserve —
//! `remove` drops the removed element when the returned value dies, `clear` runs destructors in
//! ascending index order, and an out-of-bounds `remove` panics **before** any mutation, so the
//! abort keeps the still-owned elements' destructors from ever printing. The panic reaches the
//! prelude's handler, which prints this file and the panicking line; the runner requires the
//! non-zero exit (`.expect_abort`).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;

struct Noisy(u32);

impl Drop for Noisy {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut values = Vec::new();
    values.push(Noisy(1));
    values.push(Noisy(2));
    values.push(Noisy(3));
    print_usize(values.len());

    // The removed element's destructor runs when the unused return value dies.
    values.remove(1);
    print_usize(values.len());

    // `clear` drops the survivors in ascending index order: 1 then 3.
    values.clear();
    print_usize(values.len());

    values.push(Noisy(4));
    values.push(Noisy(5));
    print_usize(values.len());

    // Not reached past the access: the out-of-bounds `remove` panics before mutating anything,
    // the handler prints the location below, and the abort means 4 and 5 never print.
    values.remove(9);
    print_u32(999);
}
