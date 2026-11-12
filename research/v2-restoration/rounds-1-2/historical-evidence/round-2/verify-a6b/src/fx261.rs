//! Statics that refer to each other and hold interior mutability: a write through one way of
//! reaching a member is read back through the other.
//!
//! The cycle is initialized in two steps (see `260_static_cycles`): each member is declared as an
//! empty object, then its value is copied onto it. That keeps the object every other binding and
//! every function names, so a cell written through `C1.next` is the cell `C2` holds. Pinned: an
//! `AtomicU32` in each member of a two-node cycle, `Cell`s inside two `static mut`s that point at
//! each other through raw pointers, and a `static mut` whose raw pointer names itself.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::sync::atomic::{AtomicU32, Ordering};

struct Counted {
    hits: AtomicU32,
    next: &'static Counted,
}
static C1: Counted = Counted { hits: AtomicU32::new(1), next: &C2 };
static C2: Counted = Counted { hits: AtomicU32::new(2), next: &C1 };

struct Cells {
    value: Cell<u32>,
    peer: *const Cells,
}
unsafe impl Sync for Cells {}
static mut M1: Cells = Cells { value: Cell::new(30), peer: &raw const M2 };
static mut M2: Cells = Cells { value: Cell::new(40), peer: &raw const M1 };

struct Selfish {
    value: Cell<u32>,
    me: *const Selfish,
}
static mut ME: Selfish = Selfish { value: Cell::new(5), me: &raw const ME };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    C1.next.hits.fetch_add(10, Ordering::Relaxed);
    C1.next.next.hits.fetch_add(100, Ordering::Relaxed);
    print_u32(C1.hits.load(Ordering::Relaxed));
    print_u32(C2.hits.load(Ordering::Relaxed));
    print_bool(core::ptr::eq(C2.next, &C1));
    unsafe {
        let m1 = &*(&raw const M1);
        let peer = &*m1.peer;
        peer.value.set(peer.value.get() + 1);
        let back = &*peer.peer;
        back.value.set(back.value.get() * 2);
        print_u32((*(&raw const M2)).value.get());
        print_u32((*(&raw const M1)).value.get());
        print_bool(core::ptr::eq(peer.peer, &raw const M1));
        let me = &*(&raw const ME);
        (*me.me).value.set(77);
        print_u32(me.value.get());
        print_bool(core::ptr::eq(me.me, &raw const ME));
    }
}
