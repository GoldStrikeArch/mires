//! will_wake on the noop waker (pointer identity of a promoted constant; control for cl5a).
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::task::Waker;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let n = Waker::noop();
    let m = Waker::noop();
    print_bool(n.will_wake(m));
    print_bool(core::ptr::eq(n.vtable(), m.vtable()));
    let k = n.clone();
    print_bool(k.will_wake(n));
    print_bool(n.will_wake(n));
}
