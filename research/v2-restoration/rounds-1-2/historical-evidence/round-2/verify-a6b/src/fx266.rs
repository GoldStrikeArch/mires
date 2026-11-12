//! Splits that would read a binding before its file is evaluated are refused at link time.
//!
//! Both statics here are `#[no_mangle]`, so a build may name either as a chunk's entry point
//! (`266_chunk_static_entry_refusals.chunks`, run by `scripts/chunk-test.sh`):
//!
//! * `s:ENTRY_S` puts `ENTRY_S` in `s.js`. It names the shared `LEAF` and the shared `HOLD` names
//!   it, so `s.js` and the shared chunk import each other and both imports are read by an
//!   initializer. Whichever file a host loads first runs second, and one read would find its
//!   binding uninitialized: refused, because only functions may be imported around a cycle of
//!   chunks.
//! * `c:ENTRY_C` would split the cycle of statics `ENTRY_C <-> OTHER` across `c.js` and the shared
//!   chunk: refused with the cycle's own message.
//!
//! Split only by its functions, and unsplit, the program runs.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct Leaf {
    n: u32,
}
static LEAF: Leaf = Leaf { n: 3 };

struct Node {
    id: u32,
    leaf: &'static Leaf,
}
#[unsafe(no_mangle)]
pub static ENTRY_S: Node = Node { id: 1, leaf: &LEAF };

struct Hold {
    n: &'static Node,
}
static HOLD: Hold = Hold { n: &ENTRY_S };

struct Ring {
    id: u32,
    next: &'static Ring,
}
#[unsafe(no_mangle)]
pub static ENTRY_C: Ring = Ring { id: 1, next: &OTHER };
static OTHER: Ring = Ring { id: 2, next: &ENTRY_C };

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_u32(HOLD.n.id + HOLD.n.leaf.n * 10);
    print_u32(OTHER.next.id + ENTRY_C.next.id * 10);
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_u32(HOLD.n.leaf.n + LEAF.n * 10);
    print_u32(OTHER.next.next.id);
    print_bool(core::ptr::eq(OTHER.next, &ENTRY_C));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
}
