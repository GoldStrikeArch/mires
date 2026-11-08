//! `BTreeMap` retypes its common leaf prefix as an internal node after a split.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::collections::BTreeMap;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut values = BTreeMap::new();
    for key in (0_u32..32).rev() {
        values.insert(key, key * 3 + 1);
    }

    let sum = values.values().copied().sum::<u32>();
    print_usize(values.len());
    print_u32(values[&0]);
    print_u32(values[&31]);
    print_u32(sum);
    print_u32(values.remove(&7).unwrap_or_default());
    print_usize(values.len());
    print_bool(values.contains_key(&7));
}
