#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
trait Speak { fn n(&self) -> u32; }
impl Speak for u32 { fn n(&self) -> u32 { *self } }
static TABLE: [Option<&(dyn Speak + Sync)>; 3] = [Some(&5u32), None, Some(&9u32)];
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut i = 0; while i < 3 { print_u32(TABLE[black_box(i)].map_or(0, |d| d.n())); i += 1; }
}
