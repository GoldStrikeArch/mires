//! A gap that is a run time refusal rather than a zombie: reaching a heap block after it is freed.
//!
//! `dealloc` unregisters the block and poisons it -- the array is emptied, so every stale pointer
//! into it reads `undefined` rather than the value it used to hold (`CONTRACT.md`, "Allocation").
//! That alone would be a silent wrong answer, so the block is also remembered as freed, and the
//! next cast through a pointer into it says so.
//!
//! The cast is where the refusal lands because that is where the model is asked a question it can
//! answer: a pointer arriving at `__rt.unscale` names a buffer, and whether that buffer is a live
//! block is a fact the runtime has. A bare dereference is not checked, and is not meant to be.
//!
//! If a later round makes freed memory readable, this test starts failing by *succeeding*.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{Layout, alloc, dealloc};

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let layout = unsafe { Layout::from_size_align_unchecked(4, 4) };
    let block = unsafe { alloc(layout) };

    let word = block as *mut i32;
    unsafe { *word = 5 };
    print_i32(unsafe { *word });

    unsafe { dealloc(block, layout) };

    // The block is gone, and the cast through a pointer into it says so.
    let again = block as *mut i32;
    print_i32(unsafe { *again });

    // Unreachable: the cast above throws.
    print_i32(999);
}
