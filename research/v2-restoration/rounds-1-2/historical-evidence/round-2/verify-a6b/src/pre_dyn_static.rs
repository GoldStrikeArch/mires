#![no_std]
#![no_main]
#![allow(dead_code, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// Pre-existing on main: a `&'static dyn Trait` inside a `static` initializer is printed as the
// thin pointer alone, without its vtable, so the first method call reads `undefined`.
// No cycle, no chunks.
trait Tr: Sync { fn v(&self) -> u32; }
struct Imp(u32);
impl Tr for Imp { fn v(&self) -> u32 { self.0 } }
static D: Imp = Imp(5);
struct Holder { d: &'static dyn Tr }
static H: Holder = Holder { d: &D };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(H.d.v());
}
