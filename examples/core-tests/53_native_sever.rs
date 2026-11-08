//! A pinned caller beside a clean native Vec region that shares its drop glue.
//!
//! `pinned_total` observes its vector's address, which pins that value to the baseline
//! representation. Both vectors run through one monomorphic drop glue, so the pin used to reach
//! the clean chain through the glue's parameter. The call-boundary model keeps the pin on the
//! pinned caller's side: the clean grow-and-read chain still receives native variants while the
//! pinned caller keeps calling the preserved baseline originals.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

#[path = "prelude.rs"]
mod prelude;

#[inline(never)]
fn shared_first(values: &Vec<u32>) -> u32 {
    values[0]
}

#[no_mangle]
pub fn clean_total(seed: u32) -> u32 {
    let mut values = Vec::with_capacity(2);
    values.push(seed.wrapping_add(3));
    values.push(seed.wrapping_mul(2));
    shared_first(&values).wrapping_add(values[1])
}

#[no_mangle]
pub fn pinned_total(seed: u32) -> u32 {
    let values: Vec<u32> = Vec::new();
    let address = values.as_ptr() as usize as u32;
    seed.wrapping_add(address & 0)
}

#[no_mangle]
pub fn rust_entry() {}
