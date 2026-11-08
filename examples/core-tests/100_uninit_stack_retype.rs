//! Stable sorting uses an aligned stack byte buffer as uninitialized element storage.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;

#[derive(Clone, Copy)]
struct Row {
    key: u32,
    ordinal: u32,
    left: u32,
    right: u32,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut rows = Vec::new();
    for ordinal in 0..48 {
        rows.push(Row {
            key: (ordinal * 17) % 13,
            ordinal,
            left: ordinal + 1,
            right: ordinal + 2,
        });
    }

    rows.sort_by(|left, right| left.key.cmp(&right.key));

    let mut sorted = true;
    let mut stable = true;
    let mut checksum = 0;
    for index in 0..rows.len() {
        let row = rows[index];
        checksum += row.ordinal;
        if index > 0 {
            let previous = rows[index - 1];
            sorted &= previous.key <= row.key;
            stable &= previous.key != row.key || previous.ordinal < row.ordinal;
        }
        let _ = row.left + row.right;
    }

    print_usize(rows.len());
    print_u32(rows[0].key);
    print_u32(rows[rows.len() - 1].key);
    print_bool(sorted);
    print_bool(stable);
    print_u32(checksum);
}
