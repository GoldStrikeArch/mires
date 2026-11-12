//! (k) references LOADED through a pointer to a pointer, a slice of references, a closure
//! environment, a struct field, then cast to a raw pointer: the raw pointer names the pointee,
//! not the place the reference was loaded from. `&*p` of a raw slot is `p`.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
struct Refs<'a> {
    r: &'a H,
    n: u32,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}
#[inline(never)]
fn load(pp: *const &H) -> *const H {
    unsafe { *pp as *const H }
}
#[inline(never)]
fn load_ref<'a>(rr: &&'a H) -> *const H {
    *rr as *const H
}
#[inline(always)]
fn load_inl(pp: *const &H) -> *const H {
    unsafe { *pp as *const H }
}

#[inline(never)]
fn t_ptrptr() {
    print_str("ptrptr");
    let h1 = mk(1);
    let h2 = mk(2);
    let mut r: &H = &h1;
    let pp: *mut &H = &mut r;
    let raw = unsafe { *pp } as *const H;
    let raw_b = load(pp);
    let raw_c = load_inl(pp);
    let raw_d = unsafe { &**pp as *const H };
    unsafe { *pp = &h2 };
    print_u32(unsafe { (*raw).head * 1000 + (*raw_b).head * 100 + (*raw_c).head * 10 + (*raw_d).head });
    print_bool(ptr::eq(raw, &h1) && ptr::eq(raw_b, &h1) && ptr::eq(raw_c, &h1) && ptr::eq(raw_d, &h1));
    print_bool(ptr::eq(unsafe { *pp }, &h2));
    print_u32(h1.head * 10 + h2.head);
    let rr: &&H = &&h1;
    let raw_e = load_ref(rr);
    let raw_f = *rr as *const H;
    print_bool(ptr::eq(raw_e, &h1) && ptr::eq(raw_f, raw_e));
    print_bool(raw_e as usize == &h1 as *const H as usize);
}

#[inline(never)]
fn t_containers() {
    print_str("containers");
    let h1 = mk(1);
    let h2 = mk(2);
    let h3 = mk(3);
    let mut v: Vec<&H> = vec![&h1, &h2];
    let raw0 = v[0] as *const H;
    let raw1 = *v.last().unwrap() as *const H;
    v[0] = &h3;
    v.swap(0, 1);
    print_u32(unsafe { (*raw0).head * 10 + (*raw1).head });
    print_bool(ptr::eq(raw0, &h1) && ptr::eq(raw1, &h2));
    let mut arr: [&H; 2] = black_box([&h1, &h2]);
    let ra = arr[1] as *const H;
    let rb = black_box(arr[0]) as *const H;
    arr = [&h3, &h3];
    print_u32(unsafe { (*ra).head * 10 + (*rb).head } + arr[0].head * 100);
    print_bool(ptr::eq(ra, &h2) && ptr::eq(rb, &h1));
    let mut refs = black_box(Refs { r: &h1, n: 1 });
    let rf = refs.r as *const H;
    let rm = &mut refs;
    let rg = rm.r as *const H;
    rm.r = &h2;
    print_u32(unsafe { (*rf).head * 10 + (*rg).head } + refs.n * 100);
    print_bool(ptr::eq(rf, &h1) && ptr::eq(rg, rf) && ptr::eq(refs.r, &h2));
    let cap: &H = &h1;
    let f = move || cap as *const H;
    let g = || cap as *const H as usize;
    print_bool(ptr::eq(f(), &h1) && g() == &h1 as *const H as usize);
    print_u32(h1.head * 100 + h2.head * 10 + h3.head);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_ptrptr();
    t_containers();
}
