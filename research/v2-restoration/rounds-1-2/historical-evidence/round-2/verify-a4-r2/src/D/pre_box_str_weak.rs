#![no_std]
#![no_main]
#![allow(unused)]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use alloc::boxed::Box;
use alloc::rc::Weak;
use core::hint::black_box;

// Two crashes met while probing Option<T> of standard types; neither involves a constant.
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let w: Option<Weak<u32>> = Some(Weak::new());
    print_u32(match black_box(&w) { Some(v) => v.strong_count() as u32 + 1, None => 0 });
    let b: Option<Box<str>> = Some("abcd".into());
    print_u32(match black_box(&b) { Some(v) => v.len() as u32, None => 0 });
    drop(b);
    print_u32(1);
}
