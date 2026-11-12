//! Reconstructed from the complete dev12 fixture read in the conversation.
//! Test operations retained; comments and formatting are new. Not hash-certified original bytes.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;
use prelude::*;
#[repr(C)]
#[derive(Clone, Copy, PartialEq)]
struct P { x: u32, y: u32 }
#[repr(C)]
struct R<'a> { n: u32, r: &'a P }
#[repr(C)]
struct R2<'a> { n: u32, r: &'a P, s: &'a P }
#[repr(C)]
struct RM<'a> { n: u32, r: &'a mut P }
#[repr(C)]
struct RO<'a> { n: u32, o: Option<&'a P> }
#[repr(C)]
struct RB { n: u32, b: Box<P> }
#[inline(never)]
fn mkp(x: u32) -> P { black_box(P { x, y: x + 1 }) }
#[inline(never)]
fn addr_of<T>(p: *const T) -> usize { p as usize }
#[inline(never)]
fn e_ref_field_eq() {
    let h = mkp(1);
    let a = black_box(R { n: 1, r: &h });
    print_bool(ptr::eq(a.r, &h));
    print_bool(a.r as *const P == &h as *const P);
    print_bool(a.r as *const P as usize == &h as *const P as usize);
    print_bool(&a.r as *const &P as usize != &h as *const P as usize);
    print_bool(addr_of(a.r) == addr_of(&h));
    print_bool(ptr::addr_eq(a.r, &h));
    print_u32(unsafe { (*(a.r as *const P)).x } + a.n);
}
#[inline(never)]
fn e_ref_field_retargeted() {
    let h = mkp(1);
    let h2 = mkp(2);
    let mut a = black_box(R { n: 1, r: &h });
    let x = a.r as *const P as usize;
    a.r = black_box(&h2);
    let y = a.r as *const P as usize;
    print_bool(x != y);
    print_bool(&h as *const P as usize != &h2 as *const P as usize);
    print_bool(y == &h2 as *const P as usize);
    print_bool(x == &h as *const P as usize);
    print_bool(ptr::eq(a.r, &h2));
    print_bool(!ptr::eq(a.r, &h));
    print_u32(a.r.x + a.n);
}
#[inline(never)]
fn e_read_through_after_retarget() {
    let h = mkp(1);
    let h2 = mkp(2);
    let mut a = black_box(R { n: 1, r: &h });
    let p = a.r as *const P;
    a.r = black_box(&h2);
    print_u32(unsafe { (*p).x });
    print_u32(a.r.x);
    print_bool(ptr::eq(p, &h));
}
#[inline(never)]
fn e_two_ref_fields() {
    let h = mkp(1);
    let h2 = mkp(2);
    let a = black_box(R2 { n: 1, r: &h, s: &h2 });
    print_bool(a.r as *const P as usize != a.s as *const P as usize);
    print_bool(!ptr::eq(a.r, a.s));
    print_bool(&a.r as *const &P as usize == &a.r as *const &P as usize);
    let b = black_box(R2 { n: 2, r: &h, s: &h });
    print_bool(ptr::eq(b.r, b.s));
    print_bool(b.r as *const P as usize == b.s as *const P as usize);
    print_bool(b.r as *const P as usize == a.r as *const P as usize);
}
#[inline(never)]
fn e_ref_elements() {
    let h = mkp(1);
    let h2 = mkp(2);
    let mut refs: [&P; 2] = black_box([&h, &h2]);
    print_bool(ptr::eq(refs[0], &h));
    print_bool(refs[1] as *const P as usize == &h2 as *const P as usize);
    print_bool(refs[0] as *const P as usize != refs[1] as *const P as usize);
    let x = refs[0] as *const P as usize;
    refs[0] = black_box(&h2);
    print_bool(refs[0] as *const P as usize != x);
    print_bool(&h as *const P as usize != &h2 as *const P as usize);
    let v: Vec<&P> = vec![&h, &h2];
    print_bool(ptr::eq(v[1], &h2));
    print_bool(v[0] as *const P as usize == &h as *const P as usize);
    let mut n = 0;
    for r in &v { if ptr::eq(*r, &h) { n += 1; } }
    print_u32(n);
}
#[inline(never)]
fn e_ref_behind_ref() {
    let h = mkp(1);
    let h2 = mkp(2);
    let mut a = black_box(R { n: 1, r: &h });
    let pp: &&P = &a.r;
    print_bool(ptr::eq(*pp, &h));
    print_bool(*pp as *const P as usize == &h as *const P as usize);
    let x = *pp as *const P as usize;
    a.r = black_box(&h2);
    let pp2: &&P = &a.r;
    print_bool(*pp2 as *const P as usize != x);
    print_bool(&h as *const P as usize != &h2 as *const P as usize);
    print_bool(ptr::eq(*pp2, &h2));
    let r: &P = &h;
    let rr: &&P = &r;
    print_bool(ptr::eq(*rr, &h));
    print_bool(*rr as *const P as usize == &h as *const P as usize);
}
#[inline(never)]
fn e_option_ref_field() {
    let h = mkp(1);
    let h2 = mkp(2);
    let mut a = black_box(RO { n: 1, o: Some(&h) });
    print_bool(ptr::eq(a.o.unwrap(), &h));
    print_bool(a.o.unwrap() as *const P as usize == &h as *const P as usize);
    let x = a.o.unwrap() as *const P as usize;
    a.o = black_box(Some(&h2));
    print_bool(a.o.unwrap() as *const P as usize != x);
    print_bool(&h as *const P as usize != &h2 as *const P as usize);
    print_bool(&a.o as *const Option<&P> as usize != &h as *const P as usize);
}
#[inline(never)]
fn e_box_field() {
    let a = black_box(RB { n: 1, b: Box::new(mkp(1)) });
    let inner: *const P = &*a.b;
    print_bool(ptr::eq(&*a.b, inner));
    print_bool(&*a.b as *const P as usize == inner as usize);
    print_bool(&a.b as *const Box<P> as usize != inner as usize);
    print_bool(ptr::eq(a.b.as_ref(), inner));
    print_u32(unsafe { (*inner).x } + a.n);
}
#[inline(never)]
fn e_mut_ref_field_write() {
    let mut h = mkp(1);
    {
        let a = black_box(RM { n: 1, r: &mut h });
        let p: *mut P = a.r;
        unsafe { (*p).x = 9; }
        print_u32(a.r.x);
    }
    print_u32(h.x);
}
#[inline(never)]
fn e_copy_through_ref_field() {
    let h = mkp(7);
    let a = black_box(R { n: 1, r: &h });
    let mut dst = mkp(0);
    unsafe { ptr::copy_nonoverlapping(a.r as *const P, &mut dst, 1); }
    print_u32(dst.x + dst.y);
    let b = unsafe { ptr::read(a.r as *const P) };
    print_u32(b.x);
}
#[inline(never)]
fn e_sort_by_ref_address() {
    let h = mkp(1);
    let h2 = mkp(2);
    let h3 = mkp(3);
    let mut v: Vec<&P> = vec![&h3, &h, &h2];
    v.sort_by_key(|r| *r as *const P as usize);
    let mut seen = 0u32;
    for (i, r) in v.iter().enumerate() {
        if ptr::eq(*r, &h) { seen += 1 << (i * 4); }
        if ptr::eq(*r, &h2) { seen += 2 << (i * 4); }
        if ptr::eq(*r, &h3) { seen += 3 << (i * 4); }
    }
    let one = (seen & 15) + ((seen >> 4) & 15) + ((seen >> 8) & 15);
    print_u32(one);
    let a = v[0] as *const P as usize;
    let b = v[1] as *const P as usize;
    let c = v[2] as *const P as usize;
    print_bool(a < b && b < c);
    print_bool((v[0] as *const P) < (v[1] as *const P));
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_ref_field_eq();
    e_ref_field_retargeted();
    e_read_through_after_retarget();
    e_two_ref_fields();
    e_ref_elements();
    e_ref_behind_ref();
    e_option_ref_field();
    e_box_field();
    e_mut_ref_field_write();
    e_copy_through_ref_field();
    e_sort_by_ref_address();
}
