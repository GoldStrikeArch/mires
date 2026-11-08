//! `chunks_exact`-shaped indexing whose dominated bounds asserts vanish.
//!
//! `fold` walks whole chunks of four under one guard per chunk: `i + 4 <= len` dominates the
//! four element reads, so the range pass proves each `s[i + k]` in bounds (the loop keeps `i`
//! under the slice-length cap, which certifies the `+ k` arithmetic cannot wrap) and deletes
//! the bounds asserts. The remainder loop re-checks `i < len` per element and those asserts go
//! the same way. The output is mode-identical; `slice_index_len_fail` stays absent, as the
//! chunked shape never range-indexes.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn fold(s: &[u8]) -> u32 {
    let mut acc = 0x811c_9dc5u32;
    let mut i = 0usize;
    let n = s.len();
    while i + 4 <= n {
        let word = (s[i] as u32)
            | ((s[i + 1] as u32) << 8)
            | ((s[i + 2] as u32) << 16)
            | ((s[i + 3] as u32) << 24);
        acc = (acc ^ word).wrapping_mul(0x0100_0193);
        i += 4;
    }
    while i < n {
        acc = (acc ^ s[i] as u32).wrapping_mul(0x0100_0193);
        i += 1;
    }
    acc
}

#[no_mangle]
pub fn rust_entry() {
    let data = [
        0x01u8, 0x7f, 0x80, 0xff, 0x10, 0x20, 0x30, 0x40, 0xaa, 0xbb, 0xcc,
    ];
    print_u32(fold(&data));
    print_u32(fold(&data[..8]));
    print_u32(fold(&data[..3]));
    print_u32(fold(&[]));
}
