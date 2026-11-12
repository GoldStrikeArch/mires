#![no_std]
#![no_main]
#![allow(unused, static_mut_refs, dangerous_implicit_autorefs)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
#[inline(never)] fn sum(s: &[u32]) -> u32 { let mut t = 0; for v in s { t = t * 3 + *v; } t }
#[inline(never)] fn show(s: &[u32]) { print_usize(s.len()); print_u32(sum(s)); }
#[inline(never)] fn showo(o: Option<&[u32]>) { match o { Some(s) => show(s), None => print_str("none") } }

static ARR: [u32; 4] = [10, 20, 30, 40];
static SL: &[u32] = &ARR;
static TAIL: &[u32] = ARR.split_at(1).1;
#[inline(never)] fn rt_whole() -> &'static [u32] { &ARR }
#[inline(never)] fn rt_tail(n: usize) -> &'static [u32] { &ARR[n..] }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let a = rt_whole(); let b = rt_tail(black_box(1));
    show(a); show(b);
    print_bool(core::ptr::eq(a, SL)); print_bool(core::ptr::eq(b, TAIL)); print_bool(a == SL); print_bool(b == TAIL);
    print_bool(core::ptr::eq(a.as_ptr(), SL.as_ptr())); print_bool(core::ptr::eq(b.as_ptr(), TAIL.as_ptr()));
    print_bool(core::ptr::eq(&a[3], &TAIL[2]));
    let o: Option<&[u32]> = Some(rt_tail(black_box(2))); showo(o);
}
