//! New regression reconstructed from the dev12 audit, not original archived bytes.
#![no_std]
#![no_main]
#[path = "prelude.rs"] mod prelude;
use prelude::*;
use core::hint::black_box;
extern crate alloc;use alloc::vec::Vec;
struct Z;impl Drop for Z {fn drop(&mut self){print_str("drop");}}
#[unsafe(no_mangle)] pub fn rust_entry(){
 let mut values=Vec::new();values.push(black_box([Z,Z]));
 drop(black_box(values));print_str("done");
}
