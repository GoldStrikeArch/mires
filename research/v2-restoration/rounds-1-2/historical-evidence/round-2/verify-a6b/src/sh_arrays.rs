#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct E { id: u32, all: &'static [E; 3], peer: &'static E }
static ARR: [E; 3] = [
    E { id: 1, all: &ARR, peer: &LONE },
    E { id: 2, all: &ARR, peer: &LONE },
    E { id: 3, all: &ARR, peer: &LONE },
];
static LONE: E = E { id: 9, all: &ARR, peer: &LONE };
struct N { id: u32, table: &'static [&'static N; 40], sl: &'static [&'static N], grid: &'static [[&'static N; 2]; 2] }
static T: [&'static N; 40] = [&N0; 40];
static G: [[&'static N; 2]; 2] = [[&N0, &N1], [&N1, &N0]];
static SL: &[&N] = &[&N1, &N0, &N1];
static N0: N = N { id: 4, table: &T, sl: &[&N1], grid: &G };
static N1: N = N { id: 6, table: &T, sl: &[], grid: &G };
struct Z;
static ZS: Z = Z;
struct HZ { z: &'static Z, u: &'static (), me: &'static HZ, e: [&'static HZ; 0], id: u8 }
static UNIT: () = ();
static HZ1: HZ = HZ { z: &ZS, u: &UNIT, me: &HZ1, e: [], id: 77 };
static OPTS: [Option<&'static N>; 3] = [Some(&N0), None, Some(&N1)];
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut t = 0;
    for e in ARR.iter() { t = t * 10 + e.id + e.all[2].id + e.peer.id; }
    print_u32(t);
    print_usize(ARR.len() + ARR[0].all.len() * 10);
    print_bool(core::ptr::eq(ARR[1].all, &ARR));
    print_bool(core::ptr::eq(&ARR[1].all[1], &ARR[1]));
    print_bool(core::ptr::eq(LONE.peer.peer, &LONE));
    print_bool(core::ptr::eq(LONE.all, &ARR));
    let s: &[E] = &ARR[1..];
    print_u32(s[0].id * 10 + s[1].all[0].id + s.len() as u32 * 100);
    let mut u = 0u32;
    for n in T.iter() { u += n.id; }
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
    print_usize(HZ1.e.len() + HZ1.me.id as usize);
    let mut k = 0;
    for o in OPTS.iter() { k = k * 10 + match o { Some(n) => n.id, None => 1 }; }
    print_u32(k);
}
