//! (s) a by-value aggregate PARAMETER whose address is taken and that is assigned again whole;
//! a `static mut` aggregate assigned whole; a by-value `self`.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
struct N {
    head: u32,
    tail: u32,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
#[inline(never)]
fn mkn(head: u32) -> N {
    black_box(N { head, tail: head + 1 })
}
fn a<T>(r: &T) -> usize {
    r as *const T as usize
}
#[inline(never)]
fn by_value(mut s: H, v: H) -> u32 {
    let p = &raw const s;
    let before = a(&s);
    s = v;
    let same = before == a(&s);
    unsafe { (*p).head * 100 + (*p).tail * 10 + same as u32 }
}
#[inline(never)]
fn by_value_field(mut s: N, v: N) -> u32 {
    let p = &raw const s.tail;
    let before = a(&s);
    s = v;
    let same = before == a(&s);
    unsafe { *p * 10 + same as u32 }
}
#[inline(never)]
fn by_value_moved(mut s: N, v: N) -> u32 {
    let before = a(&s);
    let old = s;
    s = v;
    let same = before == a(&s);
    old.head * 1000 + s.head * 10 + same as u32
}
impl N {
    #[inline(never)]
    fn replace_self(mut self, v: N) -> u32 {
        let p = &raw const self;
        self = v;
        unsafe { (*p).head }
    }
}
static mut G: H = H { head: 1, tail: 2 };
#[inline(never)]
fn global() -> u32 {
    unsafe {
        let p = &raw const G;
        let before = p as usize;
        G = mk(7);
        let same = before == &raw const G as usize;
        (*p).head * 10 + same as u32
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(by_value(mk(1), mk(5)));
    print_u32(by_value_field(mkn(1), mkn(5)));
    print_u32(by_value_moved(mkn(1), mkn(5)));
    print_u32(mkn(1).replace_self(mkn(6)));
    print_u32(global());
}
