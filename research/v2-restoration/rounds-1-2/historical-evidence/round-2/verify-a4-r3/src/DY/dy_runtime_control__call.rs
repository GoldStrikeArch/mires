//! runtime_control: call
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



#[unsafe(no_mangle)]
pub fn rust_entry() {
    let d: &dyn Tr = black_box(&B); print_u32(d.id()); let o: Option<&dyn Tr> = Some(&P); print_u32(o.map_or(0, |d| d.id()));
}
