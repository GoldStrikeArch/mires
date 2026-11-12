//! (k2) `&*p as *const T` of a raw slot `p` is `p` (CONTRACT.md, "One object per place"):
//! equality with `p`, then pointer arithmetic from the reborrowed pointer.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use core::hint::black_box;
use core::ptr;
use prelude::*;

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
#[inline(never)]
fn mk(head: u32) -> H {
    black_box(H { head, tail: head + 1 })
}

#[inline(never)]
fn t_stack() {
    print_str("stack");
    let mut hs = [mk(1), mk(2), mk(3)];
    let p: *mut H = hs.as_mut_ptr();
    let p1 = unsafe { p.add(1) };
    let again = unsafe { &*p1 as *const H };
    let again_mut = unsafe { &mut *p1 as *mut H };
    print_bool(ptr::eq(again, p1));
    print_bool(ptr::eq(again_mut, p1));
    print_bool(again as usize == p1 as usize);
    unsafe { *again_mut = mk(7) };
    print_u32(unsafe { (*p1).head });
    print_bool(unsafe { again.add(1) } == unsafe { p.add(2) });
    print_u32(unsafe { (*again.add(1)).head * 10 + (*again.sub(1)).head });
}

#[inline(never)]
fn t_heap() {
    print_str("heap");
    let b = alloc::boxed::Box::new(mk(4));
    let bp: *const H = &*b;
    let bq = unsafe { &*bp as *const H };
    print_bool(ptr::eq(bp, bq) && ptr::eq(bq, &*b));
    print_u32(unsafe { (*bq).head });
    let mut v = alloc::vec![mk(1), mk(2), mk(3)];
    let p = v.as_mut_ptr();
    let p1 = unsafe { p.add(1) };
    let again = unsafe { &*p1 as *const H };
    print_bool(ptr::eq(again, p1));
    print_bool(unsafe { again.add(1) } == unsafe { p.add(2) });
    print_u32(unsafe { (*again.add(1)).head * 10 + (*again.sub(1)).head });
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_stack();
    t_heap();
}
