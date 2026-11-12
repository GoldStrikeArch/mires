//! field_promoted: call
#![no_std]
#![no_main]
#![allow(dead_code)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

trait Tr: Sync { fn id(&self) -> u32; }
struct N { id: u32 }
impl Tr for N { fn id(&self) -> u32 { self.id } }
impl Tr for u32 { fn id(&self) -> u32 { *self } }
struct Z;
impl Tr for Z { fn id(&self) -> u32 { 77 } }
static B: N = N { id: 2 };
static P: u32 = 5;
static ZS: Z = Z;
struct Plugin { name: &'static str, imp: &'static dyn Tr, prio: u32 }

static PL: Plugin = Plugin { name: "plug", imp: &N { id: 9 }, prio: 3 };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(black_box(&PL).imp.id());
}
