#![no_std]
#![no_main]
#![allow(dead_code, unused, private_interfaces, non_upper_case_globals)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};

// `static`s of several shapes, each of which a split may name as an entry point. The shared
// function `helper` names every one of them, so the shared chunk imports every entry chunk back:
// an entry whose initializer reads a shared BINDING must be refused, one that names nothing, a
// function, itself, or another entry's static must be accepted and load in every order.
struct Leaf { n: u32 }
static LEAF: Leaf = Leaf { n: 3 };
struct Me { id: u32, me: &'static Me }
struct Ptr(*const Leaf);
unsafe impl Sync for Ptr {}

#[unsafe(no_mangle)]
pub static E_NUM: u32 = 41; // a boxed primitive: names nothing
#[unsafe(no_mangle)]
pub static E_REF: &u32 = &E_NUM; // names another entry point's static
#[unsafe(no_mangle)]
pub static E_FP: fn() -> u32 = leaf_n; // names a shared function
#[unsafe(no_mangle)]
pub static E_ARR: [&Leaf; 2] = [&LEAF, &LEAF]; // an array naming a shared binding
#[unsafe(no_mangle)]
pub static E_CNT: AtomicU32 = AtomicU32::new(7); // interior mutable: names nothing
#[unsafe(no_mangle)]
pub static E_SELF: Me = Me { id: 9, me: &E_SELF }; // a cycle of one
#[unsafe(no_mangle)]
pub static E_RAW: Ptr = Ptr(&LEAF as *const Leaf); // a raw pointer to a shared binding
#[unsafe(no_mangle)]
pub static E_UNIT: () = ();

#[inline(never)]
fn leaf_n() -> u32 { LEAF.n }
#[inline(never)]
fn helper() -> u32 {
    let unit: &() = &E_UNIT;
    E_NUM + *E_REF * 2 + E_FP() * 100 + E_ARR[1].n * 1000 + E_CNT.load(Ordering::Relaxed) * 10000
        + E_SELF.me.me.id * 100000 + (E_RAW.0.is_null() as u32) * 1000000
        + (core::ptr::eq(unit, &E_UNIT) as u32) * 10000000
}
#[unsafe(no_mangle)]
pub fn chunk_a() { print_u32(helper()); print_bool(core::ptr::eq(E_REF, &E_NUM)); }
#[unsafe(no_mangle)]
pub fn chunk_b() { print_u32(helper() + LEAF.n); print_bool(core::ptr::eq(E_SELF.me, &E_SELF)); print_bool(core::ptr::eq(E_ARR[0], &LEAF)); }
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); }
