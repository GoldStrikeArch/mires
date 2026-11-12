#![no_std]
#![no_main]
#![allow(dead_code, unused_imports)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
trait Named: Sync { fn id(&self) -> u32; }
struct N { id: u32 }
impl Named for N { fn id(&self) -> u32 { self.id } }
static B: N = N { id: 2 };
const CO: Option<&'static dyn Named> = Some(&B);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(CO.map_or(0, |n| n.id())); print_bool(black_box(CO).is_some());
}
