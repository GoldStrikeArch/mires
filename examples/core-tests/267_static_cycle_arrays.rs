//! Array statics in cycles are real arrays of the right length.
//!
//! A cycle is initialized in two steps (`260_static_cycles`): each member is first declared empty,
//! and the empty value has to be an array when the member's literal is one, or what the program
//! later does with it as an array breaks. Pinned here through operations that need a real
//! JavaScript array, not only indexing: copying an array static by value and changing the copy
//! (the copy is `.slice()`), passing one by value, copying an array of structs (a `.map`), taking a
//! subslice at a run time offset, iterating backwards, `map`. The shapes: an array whose elements
//! name the array (`ARR`), a 40-element table named back by its elements (`T`), a 2x2 grid, a
//! static slice of references, an empty array and zero-sized statics in a self-referencing
//! static.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[derive(Clone, Copy)]
struct E {
    id: u32,
    all: &'static [E; 3],
    peer: &'static E,
}
static ARR: [E; 3] = [
    E { id: 1, all: &ARR, peer: &LONE },
    E { id: 2, all: &ARR, peer: &LONE },
    E { id: 3, all: &ARR, peer: &LONE },
];
static LONE: E = E { id: 9, all: &ARR, peer: &LONE };

struct N {
    id: u32,
    table: &'static [&'static N; 40],
    sl: &'static [&'static N],
    grid: &'static [[&'static N; 2]; 2],
}
static T: [&N; 40] = [&N0; 40];
static G: [[&N; 2]; 2] = [[&N0, &N1], [&N1, &N0]];
static SL: &[&N] = &[&N1, &N0, &N1];
static N0: N = N { id: 4, table: &T, sl: &[&N1], grid: &G };
static N1: N = N { id: 6, table: &T, sl: &[], grid: &G };

struct Z;
static ZS: Z = Z;
static UNIT: () = ();
struct HZ {
    z: &'static Z,
    u: &'static (),
    me: &'static HZ,
    e: [&'static HZ; 0],
    id: u8,
}
static HZ1: HZ = HZ { z: &ZS, u: &UNIT, me: &HZ1, e: [], id: 77 };

#[inline(never)]
fn by_value(a: [&'static N; 40]) -> u32 {
    let mut t = 0;
    for n in a.iter() {
        t += n.id;
    }
    t * 1000 + a.len() as u32
}

#[inline(never)]
fn sum(s: &[E]) -> u32 {
    s.iter().map(|e| e.id).sum()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut t = 0;
    for e in ARR.iter() {
        t = t * 10 + e.id + e.all[2].id + e.peer.id;
    }
    print_u32(t);
    print_usize(ARR.len() + ARR[0].all.len() * 10);
    print_bool(core::ptr::eq(ARR[1].all, &ARR));
    print_bool(core::ptr::eq(&ARR[1].all[1], &ARR[1]));
    print_bool(core::ptr::eq(LONE.peer.peer, &LONE));
    print_bool(core::ptr::eq(LONE.all, &ARR));

    // By value: a copy of the static array, changed; the static is not.
    let mut local = T;
    local[0] = &N1;
    local[39] = &N1;
    print_u32(local[0].id * 100 + local[39].id * 10 + T[0].id);
    print_u32(by_value(T));
    print_u32(by_value(*N1.table));
    let mut copy = ARR;
    copy[1].id = 50;
    print_u32(copy[1].id + ARR[1].id + copy.len() as u32 * 1000);

    // A subslice at a run time offset, a backwards walk, `map`.
    let from = black_box(1usize);
    print_u32(sum(&ARR[from..]) + ARR[from..].len() as u32 * 100);
    let mut w = 0;
    for row in G.iter().rev() {
        for n in row.iter().rev() {
            w = w * 10 + n.id;
        }
    }
    print_u32(w);
    let ids = G[0].map(|n| n.id);
    print_u32(ids[0] * 10 + ids[1]);

    let mut u = 0u32;
    for n in T.iter() {
        u += n.id;
    }
    print_u32(u + T.len() as u32 * 1000);
    print_bool(core::ptr::eq(T[39], &N0));
    print_bool(core::ptr::eq(N0.table, &T));
    print_bool(core::ptr::eq(N1.table[7].table, &T));
    print_u32(G[0][1].id * 10 + G[1][1].grid[1][0].id + N0.grid[0][0].id * 100);
    print_bool(core::ptr::eq(N1.grid, &G));
    print_usize(SL.len() * 100 + N0.sl.len() * 10 + N1.sl.len());
    print_u32(SL[0].id + SL[1].sl[0].id * 10);
    print_bool(core::ptr::eq(SL[0], N0.sl[0]));
    print_bool(core::ptr::eq(HZ1.me.me, &HZ1));
    print_bool(core::ptr::eq(HZ1.z, &ZS));
    print_bool(core::ptr::eq(HZ1.u, &UNIT));
    print_usize(HZ1.e.len() + HZ1.me.id as usize);
}
