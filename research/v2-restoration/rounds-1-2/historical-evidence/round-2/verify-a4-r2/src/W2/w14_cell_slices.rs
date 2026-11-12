#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cell::Cell;
use core::hint::black_box;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

// safe Rust: statics with interior mutability reached through static slices, in a table and a struct
struct Counter { hits: AtomicUsize, on: AtomicBool }
static COUNTERS: [Counter; 3] = [
    Counter { hits: AtomicUsize::new(0), on: AtomicBool::new(false) },
    Counter { hits: AtomicUsize::new(10), on: AtomicBool::new(true) },
    Counter { hits: AtomicUsize::new(20), on: AtomicBool::new(false) },
];
static ALL: &[Counter] = &COUNTERS;
static LAST_TWO: &[Counter] = COUNTERS.split_at(1).1;
static SOLO: Counter = Counter { hits: AtomicUsize::new(100), on: AtomicBool::new(true) };
static SOLO_SL: &[Counter] = core::slice::from_ref(&SOLO);
struct Registry { name: &'static str, group: &'static [Counter], extra: Option<&'static [Counter]> }
static REG: [Registry; 2] = [
    Registry { name: "all", group: ALL, extra: Some(SOLO_SL) },
    Registry { name: "tail", group: LAST_TWO, extra: None },
];
static BYTES: [AtomicU8; 4] = [AtomicU8::new(1), AtomicU8::new(2), AtomicU8::new(3), AtomicU8::new(4)];
static BYTES_MID: &[AtomicU8] = BYTES.split_at(1).1.split_at(2).0;

#[inline(never)] fn total(s: &[Counter]) -> usize { s.iter().map(|c| c.hits.load(Ordering::Relaxed) + c.on.load(Ordering::Relaxed) as usize * 1000).sum() }
#[inline(never)] fn bump(s: &[Counter]) { for c in s { c.hits.fetch_add(1, Ordering::Relaxed); c.on.store(!c.on.load(Ordering::Relaxed), Ordering::Relaxed); } }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(total(ALL)); print_usize(total(LAST_TWO)); print_usize(total(SOLO_SL));
    bump(LAST_TWO);
    print_usize(total(&COUNTERS)); print_usize(COUNTERS[1].hits.load(Ordering::Relaxed)); print_usize(COUNTERS[0].hits.load(Ordering::Relaxed));
    bump(black_box(ALL));
    print_usize(total(LAST_TWO)); print_usize(COUNTERS[2].hits.load(Ordering::Relaxed));
    COUNTERS[2].hits.store(500, Ordering::Relaxed);
    print_usize(LAST_TWO[1].hits.load(Ordering::Relaxed)); print_usize(ALL[2].hits.load(Ordering::Relaxed));
    bump(SOLO_SL); print_usize(SOLO.hits.load(Ordering::Relaxed)); print_bool(SOLO.on.load(Ordering::Relaxed));
    SOLO.hits.store(7, Ordering::Relaxed); print_usize(SOLO_SL[0].hits.load(Ordering::Relaxed));
    for r in &REG {
        print_str(r.name); print_usize(total(r.group));
        bump(r.group);
        if let Some(e) = r.extra { bump(e); print_usize(total(e)); }
    }
    print_usize(total(&COUNTERS)); print_usize(SOLO.hits.load(Ordering::Relaxed));
    BYTES_MID[0].store(20, Ordering::Relaxed); BYTES[2].store(30, Ordering::Relaxed);
    print_u32(BYTES[1].load(Ordering::Relaxed) as u32); print_u32(BYTES_MID[1].load(Ordering::Relaxed) as u32); print_usize(BYTES_MID.len());
    print_bool(core::ptr::eq(&LAST_TWO[0], &COUNTERS[1])); print_bool(core::ptr::eq(&SOLO_SL[0], &SOLO)); print_bool(core::ptr::eq(REG[0].group, ALL));
    print_bool(core::ptr::eq(REG[1].group.as_ptr(), &COUNTERS[1])); print_bool(core::ptr::eq(BYTES_MID.as_ptr(), &BYTES[1]));
}
