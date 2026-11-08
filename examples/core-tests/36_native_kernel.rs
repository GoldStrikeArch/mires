//! Scalar-ABI leaf functions lowered by the JavaScript-native language kernel.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;

struct Pair {
    left: u32,
    right: u32,
}

#[no_mangle]
pub fn native_add(left: u32, right: u32) -> u32 {
    left + right
}

#[no_mangle]
pub fn native_pair_sum(left: u32, right: u32) -> u32 {
    let pair = Pair { left, right };
    pair.left + pair.right
}

#[no_mangle]
pub fn native_choose(yes: bool) -> u32 {
    if yes { 7 } else { 9 }
}

#[no_mangle]
pub fn rust_entry() {
    prelude::print_u32(native_add(u32::MAX, 2));
    prelude::print_u32(native_pair_sum(12, 30));
    prelude::print_u32(native_choose(true));
    prelude::print_u32(native_choose(false));
}
