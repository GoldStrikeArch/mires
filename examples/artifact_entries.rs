//! Independent roots for deployment graph and runtime-provider regression tests.

#![no_std]
#![no_main]

#[path = "core-tests/prelude.rs"]
mod prelude;

#[cfg(not(worker))]
#[inline(never)]
fn shared(value: i32) -> i32 {
    prelude::print_bool(value > 0);
    value + 1
}

#[cfg(not(worker))]
#[no_mangle]
pub fn counter() -> i32 {
    shared(1)
}

#[cfg(not(worker))]
#[no_mangle]
pub fn search() -> i32 {
    shared(2)
}

#[cfg(worker)]
#[no_mangle]
pub fn worker_entry(value: f64) -> i32 {
    value as i32
}
