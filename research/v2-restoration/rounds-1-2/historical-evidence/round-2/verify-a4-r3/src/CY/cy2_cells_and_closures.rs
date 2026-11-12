//! Cyclic statics holding one-element slices over interior-mutable members and closure fn pointers.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::hint::black_box;
use core::ptr;
use core::slice::from_ref;

#[derive(Clone, Copy)]
struct Pt { x: u32, y: u32 }
struct CNode { v: Cell<Pt>, peers: &'static [CNode] }
unsafe impl Sync for CNode {}
static CA: CNode = CNode { v: Cell::new(Pt { x: 1, y: 2 }), peers: from_ref(&CB) };
static CB: CNode = CNode { v: Cell::new(Pt { x: 3, y: 4 }), peers: from_ref(&CA) };

struct St { f: fn(u32) -> u32, g: Option<fn(u32, u32) -> u32>, next: &'static St, all: &'static [St] }
static S1: St = St { f: |x| x + 1, g: Some(|a, b| a * b), next: &S2, all: from_ref(&S2) };
static S2: St = St { f: |x| x * 2, g: None, next: &S1, all: from_ref(&S1) };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // whole-element stores through the cyclic slices, in both directions
    CA.peers[0].v.set(Pt { x: 30, y: 40 });
    print_u32(CB.v.get().x + CB.v.get().y);
    CA.v.set(Pt { x: 10, y: 20 });
    print_u32(CB.peers[black_box(0)].v.get().x + CB.peers[0].v.get().y);
    let old = CB.peers[0].peers[0].v.replace(Pt { x: 7, y: 8 });
    print_u32(old.x); print_u32(CB.v.get().y);
    print_bool(ptr::eq(CA.peers.as_ptr(), &CB)); print_bool(ptr::eq(CB.peers.as_ptr(), &CA));
    // closures in cyclic statics
    print_u32((S1.f)(5)); print_u32((S1.next.f)(5)); print_u32((S2.next.f)(6));
    print_u32(S1.g.map_or(0, |g| g(3, 4))); print_u32(S2.g.map_or(99, |g| g(3, 4)));
    print_u32((S1.all[0].f)(7)); print_u32((S2.all[0].all[0].f)(7));
    print_bool(ptr::eq(S1.all.as_ptr(), &S2)); print_bool(ptr::eq(S1.next, &S2)); print_bool(ptr::eq(S2.all.as_ptr(), S2.next));
}
