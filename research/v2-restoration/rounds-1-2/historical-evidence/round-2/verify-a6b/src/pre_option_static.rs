#![no_std]
#![no_main]
#![allow(dead_code, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// Pre-existing on main (the niche-constant decoder, A4): `Some(..)` of a niche-optimized Option
// inside a `static` initializer is printed as `{ TAG: "None" }`. No cycle, no chunks.
fn seven() -> u32 { 7 }
struct Leaf { n: u32 }
static LEAF: Leaf = Leaf { n: 3 };
struct Holder { r: Option<&'static Leaf>, f: Option<fn() -> u32> }
static H: Holder = Holder { r: Some(&LEAF), f: Some(seven) };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(match H.r { Some(l) => l.n, None => 0 });
    print_u32(match H.f { Some(f) => f(), None => 0 });
}
