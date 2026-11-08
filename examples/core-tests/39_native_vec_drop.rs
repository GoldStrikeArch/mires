//! An ordinary local `Vec` whose destructor runs before the exported function returns.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

#[path = "prelude.rs"]
mod prelude;

#[no_mangle]
pub fn native_vec_local_len() -> usize {
    let mut values = Vec::new();
    values.push(3_u32);
    values.push(5_u32);
    values.len()
}

#[no_mangle]
pub fn rust_entry() {}
