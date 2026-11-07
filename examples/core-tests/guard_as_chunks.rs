//! A gap that is a run time guard rather than a zombie, pinned so that it stays loud.
//!
//! `as_chunks` rebuilds a slice at *chunk* granularity: `[i32]` reinterpreted as `[[i32; 2]]`. The
//! pointer model can re-key a buffer but it cannot invent elements, and a buffer of five numbers
//! holds no two element arrays, so there is nothing to point at. Nothing in the *type* says
//! whether a given pointer to `[E; N]` is a window an earlier cast produced over somebody else's
//! buffer or a slot whose one element really is an array — so the question cannot be answered when
//! the item is lowered, and `__rt.chunk_slice` asks it when the program runs.
//!
//! What this file pins is that the answer is an exception and not a wrong number: the length
//! printed first appears, the chunking then aborts the program, and `guard_as_chunks.expect_abort`
//! requires the non-zero exit. The neighbouring guards — dereferencing a byte view, `__rt.unscale`
//! on a misaligned offset, a `copy` between two pointers whose scales disagree — are the same
//! shape and are listed in `CONTRACT.md`; `align_to`, which is how `memchr` reads a haystack a
//! `usize` at a time, is the one a program is most likely to reach by accident.
//!
//! If a later round makes `as_chunks` work, this test starts failing by *succeeding*. Replace it
//! with the ordinary expectation then.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let values = [1i32, 2, 3, 4, 5];
    print_usize(values.len());

    let (chunks, rest) = values.as_chunks::<2>();

    // Unreachable: `as_chunks` throws above.
    print_usize(chunks.len());
    print_usize(rest.len());
    print_i32(chunks[0][0]);
}
