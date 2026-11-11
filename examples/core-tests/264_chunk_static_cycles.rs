//! Statics that refer to each other, in a chunked program.
//!
//! Whichever chunk reaches one member of a cycle reaches all of them, so a cycle always has one
//! owner and is initialized in two steps in that file (see `260_static_cycles`). Split into two
//! chunks (`264_chunk_static_cycles.chunks`, run by `scripts/chunk-test.sh`), this pins: a cycle
//! both chunks reach (owned by the shared chunk, with an `AtomicU32` written from one chunk and
//! read from the other), a cycle only chunk `a` reaches, a self-referencing static in each chunk,
//! a static of chunk `a` pointing into the shared cycle, and a cycle whose members name chunk
//! `b`'s entry point (so chunk `a` imports it). The program also runs unsplit.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::sync::atomic::{AtomicU32, Ordering};

struct Node {
    id: u32,
    hits: AtomicU32,
    next: &'static Node,
}
/// Both chunks.
static SA: Node = Node { id: 1, hits: AtomicU32::new(0), next: &SB };
static SB: Node = Node { id: 2, hits: AtomicU32::new(0), next: &SA };
/// Chunk a only.
static OA: Node = Node { id: 3, hits: AtomicU32::new(0), next: &OB };
static OB: Node = Node { id: 4, hits: AtomicU32::new(0), next: &OA };
static INTO_SHARED: &Node = &SB;

struct Me {
    id: u32,
    me: &'static Me,
}
/// One self-referencing static per chunk.
static ME_A: Me = Me { id: 7, me: &ME_A };
static ME_B: Me = Me { id: 8, me: &ME_B };

struct Hop {
    id: u32,
    call: fn(),
    next: &'static Hop,
}
/// Chunk a only; names chunk b's entry point.
static HA: Hop = Hop { id: 5, call: chunk_b, next: &HB };
static HB: Hop = Hop { id: 6, call: chunk_b, next: &HA };

static DEPTH: AtomicU32 = AtomicU32::new(0);

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("chunk a");
    SA.next.hits.fetch_add(5, Ordering::Relaxed);
    print_u32(SA.id * 10 + SA.next.next.id);
    print_u32(OA.next.id * 10 + OB.next.next.id);
    print_bool(core::ptr::eq(OB.next, &OA));
    print_u32(INTO_SHARED.next.id * 10 + INTO_SHARED.next.next.id);
    print_u32(ME_A.me.me.id);
    print_bool(core::ptr::eq(ME_A.me, &ME_A));
    print_u32(HA.next.next.id + HB.next.id);
    if DEPTH.fetch_add(1, Ordering::Relaxed) < 2 {
        (HA.next.call)();
    }
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("chunk b");
    print_u32(SB.hits.load(Ordering::Relaxed));
    SB.next.next.hits.fetch_add(1, Ordering::Relaxed);
    print_bool(core::ptr::eq(SB.next.next, &SB));
    print_u32(SB.next.id);
    print_u32(ME_B.me.id);
    print_bool(core::ptr::eq(ME_B.me.me, &ME_B));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
}
