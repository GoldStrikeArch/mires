//! NEW REFUSAL of a program main compiled and ran correctly: a `static` struct with a
//! `&'static dyn Trait` field that points at another `static`, where the program reads only the
//! other fields. (Any call through the field threw a TypeError on main; that part was never right.)
//! Native (-O0, -O3): plug 3. Main (ea5604f), all four modes: plug 3.
//! e2245df, all four modes: compile error "a `&dyn Tr` in a constant is a trait object, whose
//! vtable rustc_codegen_js does not decode out of constant data".
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

trait Tr: Sync {
    fn id(&self) -> u32;
}
struct N {
    id: u32,
}
impl Tr for N {
    fn id(&self) -> u32 {
        self.id
    }
}
static B: N = N { id: 2 };
struct Plugin {
    name: &'static str,
    imp: &'static dyn Tr,
    prio: u32,
}
static PL: Plugin = Plugin { name: "plug", imp: &B, prio: 3 };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str(PL.name);
    print_u32(black_box(&PL).prio);
}
