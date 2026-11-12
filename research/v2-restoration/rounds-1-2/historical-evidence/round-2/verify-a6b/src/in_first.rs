#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// References to the FIRST field (offset 0) of a static, inside a cycle and outside one.
#[repr(C)]
struct Inner { x: u32, y: u32 }
#[repr(C)]
struct NodeA { inner: Inner, id: u32, other: &'static HolderB }
#[repr(C)]
struct HolderB { r: &'static Inner, back: &'static NodeA, id: u32 }
static IA: NodeA = NodeA { inner: Inner { x: 5, y: 6 }, id: 1, other: &IB };
static IB: HolderB = HolderB { r: &IA.inner, back: &IA, id: 2 };
// the other name order
static ZB: NodeA = NodeA { inner: Inner { x: 7, y: 8 }, id: 3, other: &ZA };
static ZA: HolderB = HolderB { r: &ZB.inner, back: &ZB, id: 4 };
// self
#[repr(C)]
struct SelfIn { inner: Inner, id: u32, me: &'static Inner, whole: &'static SelfIn }
static SI: SelfIn = SelfIn { inner: Inner { x: 9, y: 10 }, id: 5, me: &SI.inner, whole: &SI };
// no cycle at all
#[repr(C)]
struct Plain { inner: Inner, id: u32 }
static PL: Plain = Plain { inner: Inner { x: 11, y: 12 }, id: 6 };
static PR: &Inner = &PL.inner;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(PR.x * 100 + PR.y);
    print_bool(core::ptr::eq(PR, &PL.inner));
    print_u32(IB.r.x * 100 + IB.r.y + IA.other.r.y * 10000);
    print_bool(core::ptr::eq(IB.r, &IA.inner));
    print_u32(ZA.r.x * 100 + ZA.r.y + ZB.other.r.y * 10000);
    print_bool(core::ptr::eq(ZA.r, &ZB.inner));
    print_u32(SI.me.x * 100 + SI.me.y + SI.whole.me.x * 10000);
    print_bool(core::ptr::eq(SI.me, &SI.inner));
}
