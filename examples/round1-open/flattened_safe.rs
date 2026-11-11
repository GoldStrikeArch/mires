#![no_std]
#![no_main]
#![allow(warnings)]
#[path="prelude.rs"] mod prelude;
use prelude::*;

#[inline(never)] fn read(a: &[[u32; 2]]) { let flat=a.as_flattened(); for i in 0..flat.len() { print_u32(flat[i]); } }
#[unsafe(no_mangle)] pub fn rust_entry() { let a=core::hint::black_box([[1u32,2],[3,4]]); read(&a); }
