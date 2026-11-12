#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
static ARR: [u32; 3] = [100, 200, 300];
struct Holder { name: &'static str, data: Option<&'static [u32]> }
static H: Holder = Holder { name: "h", data: Some(&ARR) };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(black_box(&H).data.map_or(0, |v| v[2]));
    print_usize(H.data.map_or(0, |v| v.len()));
}
