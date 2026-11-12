//! Client crate: uses the provider's closure fn-pointer constants and one-element static slice.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use xc_lib::*;

#[inline(never)]
fn use_generic<T: Copy + Into<u64>>(t: T) -> u64 { (G::<T>::CONV)(t) }
#[inline(never)]
fn use_assoc<T: HasF>(x: u32) -> u32 { (T::F)(x) }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(C1(1)); print_u32(black_box(C1)(2)); print_u32(C0());
    print_u32(S1(1)); print_u32(black_box(&S1)(2));
    print_u32(OC.map_or(0, |f| f(1)));
    for i in 0..2 { let op = &OPS[black_box(i)]; print_str(op.name); print_u32((op.run)(3, 4)); }
    for i in 0..3 { print_u32(TABLE[black_box(i)](5)); }
    print_u64(W64(1, ()));
    print_u64(use_generic(7u8)); print_u64(use_generic(1u64 << 33));
    print_u32(use_assoc::<I1>(1));
    print_u32(get()(1)); print_u32(get_inline()(1)); print_u32(call_here(1));
    print_usize(ONE.len()); print_u32(ONE[0].id);
    print_bool(core::ptr::eq(ONE.as_ptr(), &B));
    print_bool(core::ptr::eq(one_here().as_ptr(), &B));
    print_bool(core::ptr::eq(ONE.as_ptr(), core::slice::from_ref(black_box(&B)).as_ptr()));
}
