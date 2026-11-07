//! A gap that is a run time refusal rather than a zombie: reading one heap block at two element
//! sizes.
//!
//! A block is born byte granular and is retyped exactly once, at the first cast away from `*mut u8`
//! (`CONTRACT.md`, "Allocation"). That is sound only while the block is *fresh*: its bytes are
//! uninitialized, so there is nothing in them to lose. Once something has been written at one
//! element size, the same bytes read at another are a reinterpretation the model cannot express --
//! a JavaScript array of four numbers holds no two-element run of wider ones.
//!
//! Nothing in the *type* of a cast says whether the block behind it is fresh, so the question
//! cannot be answered when the item is lowered; `__rt.unscale` asks it when the program runs. What
//! this file pins is that the answer is an exception and not a wrong number.
//!
//! If a later round makes the reinterpretation work, this test starts failing by *succeeding*.
//! Replace it with the ordinary expectation then.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::{Layout, alloc, dealloc};

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let layout = unsafe { Layout::from_size_align_unchecked(16, 4) };
    let block = unsafe { alloc(layout) };

    // The first cast retypes the block: sixteen bytes become four `i32`s.
    let words = block as *mut i32;
    unsafe { *words = 7 };
    print_i32(unsafe { *words });

    // The second asks for the same bytes as two `i64`s. The block is no longer fresh.
    let wide = block as *mut i64;
    print_i64(unsafe { *wide });

    // Unreachable: the cast above throws.
    unsafe { dealloc(block, layout) };
    print_i32(999);
}
