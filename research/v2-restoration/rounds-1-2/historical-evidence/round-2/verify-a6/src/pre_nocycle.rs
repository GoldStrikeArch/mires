#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// No cycle anywhere: probes for bugs that exist on main independently of A6.
// (1) a reference to the first field of a static (offset 0, another type)
#[repr(C)]
struct Inner { x: u32, y: u32 }
#[repr(C)]
struct Plain { inner: Inner, id: u32 }
static PL: Plain = Plain { inner: Inner { x: 11, y: 12 }, id: 6 };
static PR: &Inner = &PL.inner;
// (2) a `&dyn Trait` constant that points at a static
trait Tr: Sync { fn id(&self) -> u32; }
struct X { id: u32 }
impl Tr for X { fn id(&self) -> u32 { self.id } }
static X1: X = X { id: 1 };
static D: &dyn Tr = &X1;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(PR.x * 100 + PR.y);
    print_bool(core::ptr::eq(PR, &PL.inner));
    print_u32(D.id());
}
