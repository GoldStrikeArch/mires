//! (t) a value moved OUT of a heap block (`*boxed`), whose block is then freed and its address
//! range reused by a later allocation: the moved value is a different place from the new box.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;
use prelude::*;

struct H {
    head: u32,
    tail: u32,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
fn a(r: &H) -> usize {
    r as *const H as usize
}

#[inline(never)]
fn t_box() {
    print_str("box");
    let b = Box::new(mk(1));
    let pb = a(&b);
    let s = *b;
    let b2 = Box::new(mk(2));
    print_bool(pb != 0);
    print_bool(a(&s) != a(&b2));
    print_bool(!ptr::eq(&s, &*b2));
    print_u32(s.head * 10 + b2.head);
    let mut s2 = s;
    s2.head = 9;
    print_u32(s2.head * 10 + b2.head);
}

#[inline(never)]
fn t_vec() {
    print_str("vec");
    let mut v: Vec<H> = vec![mk(3)];
    let pv = a(&v[0]);
    let e = v.pop().unwrap();
    drop(v);
    let v2: Vec<H> = vec![mk(4)];
    print_bool(pv != 0);
    print_bool(a(&e) != a(&v2[0]));
    print_u32(e.head * 10 + v2[0].head);
    let ob = black_box(Some(Box::new(mk(5))));
    let p5 = ob.as_ref().map_or(0, |b| a(b));
    let inner = *ob.unwrap();
    let b3 = Box::new(mk(6));
    print_bool(p5 != 0);
    print_bool(a(&inner) != a(&b3));
    print_u32(inner.head * 10 + b3.head);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_box();
    t_vec();
}
