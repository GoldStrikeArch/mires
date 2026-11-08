//! New regression reconstructed from the dev12 audit, not original archived bytes.
#![no_std]
#![no_main]
#[path = "prelude.rs"] mod prelude;
use prelude::*;
use core::hint::black_box;
#[derive(Clone,Copy)]struct H{head:u8}struct Outer{n:u32,h:H}
#[unsafe(no_mangle)]pub fn rust_entry(){
 let mut o=black_box(Outer{n:17,h:H{head:11}});
 let a=&o.h as *const H as usize;o.h=black_box(H{head:12});
 let b=&o.h.head as *const u8 as usize;
 print_bool(a==&o.h as *const H as usize);
 print_bool(b==&o.h as *const H as usize);print_u32(o.n+o.h.head as u32);
}
