//! New regression reconstructed from the dev12 audit, not original archived bytes.
#![no_std]
#![no_main]
#[path = "prelude.rs"] mod prelude;
use prelude::*;
use core::hint::black_box;
#[derive(Clone,Copy)] #[repr(C)] struct P { x:u32, y:u32 }
#[unsafe(no_mangle)] pub fn rust_entry() {
 let a=black_box([P{x:1,y:2},P{x:3,y:4}]); let r=black_box(&a[1]);
 print_bool(core::ptr::eq(r,&a[1])); print_bool(core::ptr::addr_eq(r,&a[1]));
 print_bool(r as *const P as usize == &a[1] as *const P as usize);
 print_u32(r.x);
}
