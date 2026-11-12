#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
trait Speak { fn n(&self) -> u32; }
impl Speak for u32 { fn n(&self) -> u32 { *self } }
static DYN: Option<&(dyn Speak + Sync)> = None;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(black_box(&DYN).map_or(0, |d| d.n()));
}
