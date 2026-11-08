//! New regression reconstructed from the dev12 audit, not original archived bytes.
#![no_std]
#![no_main]
#[path = "prelude.rs"] mod prelude;
use prelude::*;
use core::hint::black_box;
struct Z; impl Drop for Z { fn drop(&mut self){print_str("drop");} }
#[inline(never)] fn make()->[Z;2]{[Z,Z]}
#[unsafe(no_mangle)] pub fn rust_entry(){
 let arr=black_box(make());let f=move || {black_box(&arr);};
 drop(black_box(f));print_str("done");
}
