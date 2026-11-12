#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// References into the MIDDLE of a static (a later field, an array element), inside a cycle.
#[repr(C)]
struct Inner { x: u32, y: u32 }
#[repr(C)]
struct NodeA { id: u32, inner: Inner, arr: [u32; 3], other: &'static HolderB }
#[repr(C)]
struct HolderB { id: u32, r: &'static Inner, e: &'static u32, back: &'static NodeA }
static IA: NodeA = NodeA { id: 1, inner: Inner { x: 5, y: 6 }, arr: [7, 8, 9], other: &IB };
static IB: HolderB = HolderB { id: 2, r: &IA.inner, e: &IA.arr[1], back: &IA };
static ZB: NodeA = NodeA { id: 3, inner: Inner { x: 15, y: 16 }, arr: [17, 18, 19], other: &ZA };
static ZA: HolderB = HolderB { id: 4, r: &ZB.inner, e: &ZB.arr[1], back: &ZB };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(IB.r.x * 100 + IB.r.y + *IB.e * 10000);
    print_bool(core::ptr::eq(IB.r, &IA.inner));
    print_bool(core::ptr::eq(IB.e, &IA.arr[1]));
    print_u32(ZA.r.x * 100 + ZA.r.y + *ZA.e * 10000);
    print_bool(core::ptr::eq(ZA.r, &ZB.inner));
}
