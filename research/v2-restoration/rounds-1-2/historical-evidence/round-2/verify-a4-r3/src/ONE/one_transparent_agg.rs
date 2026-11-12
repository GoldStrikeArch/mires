//! One-element slice over a `static` of element kind `transparent_agg`.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr;
use core::slice::from_ref;

struct P { x: u32 } #[repr(transparent)] struct T(P);
type T0 = T;
static S: T0 = T(P { x: 7 });
static ONE: &[T0] = from_ref(&S);
static TWO: &[T0] = from_ref(&S);
static OPT: Option<&[T0]> = Some(from_ref(&S));

#[inline(never)]
fn show(v: &T0) { print_u32(v.0.x); }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(ONE.len());
    print_usize(black_box(ONE).iter().count());
    show(&ONE[0]);
    show(&S);
    for v in TWO { show(v); }
    print_usize(OPT.map_or(9, |s| s.len()));
    if core::mem::size_of::<T0>() != 0 {
        print_bool(ptr::eq(ONE.as_ptr(), &S));
        print_bool(ptr::eq(&ONE[0], &S));
        print_bool(ptr::eq(ONE.as_ptr(), TWO.as_ptr()));
        print_bool(ptr::eq(ONE.as_ptr(), from_ref(black_box(&S)).as_ptr()));
        print_bool(OPT.map_or(false, |s| ptr::eq(s.as_ptr(), &S)));
    }
}
