#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
trait Speak { fn n(&self) -> u32; }
impl Speak for u32 { fn n(&self) -> u32 { *self } }
const DYN: &(dyn Speak + Sync) = &5u32;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(black_box(DYN).n());
}
