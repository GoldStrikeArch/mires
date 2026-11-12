//! Identity and equality of core's noop waker (its vtable is four closures in a constant).
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::task::Waker;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let n = Waker::noop();
    let m = Waker::noop();
    print_bool(n.data() == m.data());
    print_bool(*n.vtable() == *m.vtable());
    print_bool(black_box(n).vtable() == black_box(Waker::noop()).vtable());
    let k = n.clone();
    print_bool(*k.vtable() == *n.vtable());
    print_bool(*n.vtable() == *n.vtable());
    k.wake_by_ref();
    k.wake();
    print_u32(1);
}
