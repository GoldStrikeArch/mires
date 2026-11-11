//! Three chunks and the shared one, each declaring only the statics it owns.
//!
//! Split three ways (`263_chunk_three_owners.chunks`, run by `scripts/chunk-test.sh`, once with
//! the chunks asked for in each direction). Chains of statics owned by one chunk reach statics the
//! shared chunk owns (`C_CHAIN -> C_PETS -> PETS -> DOG`), a static only chunk `b` reads names a
//! static only chunk `a` reads otherwise (`B_SEES_A -> A_PET`, which makes `A_PET` shared), two
//! chunk-owned statics hold the same long string (a hoisted shared constant), and a shared static
//! names chunk `a`'s entry point, so the shared chunk imports from a chunk. Every one of those
//! names used to be declared again by the chunk that imports it. The program also runs unsplit.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::sync::atomic::{AtomicU32, Ordering};

struct Pet {
    n: u32,
    say: fn(&Pet) -> u32,
}
fn dog(p: &Pet) -> u32 {
    p.n * 3
}
fn cat(p: &Pet) -> u32 {
    p.n + 1000
}

static DOG: Pet = Pet { n: 5, say: dog };
static CAT: Pet = Pet { n: 6, say: cat };
/// Chunk a, and chunk b through `B_SEES_A`: shared.
static A_PET: &Pet = &DOG;
/// Chunks b and c: shared.
static PETS: [&Pet; 2] = [&DOG, &CAT];
/// Chunk c only: a chain into the shared table.
static C_PETS: &[&Pet; 2] = &PETS;
static C_CHAIN: &&[&Pet; 2] = &C_PETS;

static NAME_LONG: &str = "a-name-that-is-long-enough-to-be-hoisted-into-a-shared-constant";
/// Chunk a only, holds a long string.
static A_LABEL: (&str, u32) = (NAME_LONG, 1);
/// Chunk b only, holds the same long string.
static B_LABEL: (&str, u32) = (NAME_LONG, 2);

static HITS: AtomicU32 = AtomicU32::new(0);
/// Read by every chunk, so owned by the shared chunk, and it names chunk a's entry point, which
/// chunk a owns: the shared chunk imports from chunk a.
static ENTRY_TABLE: [fn(); 1] = [chunk_a];

/// Chunk b only; names a static that only chunk a would otherwise read.
static B_SEES_A: &&Pet = &A_PET;

fn say(p: &Pet) -> u32 {
    (p.say)(p)
}

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("chunk a");
    print_u32(say(A_PET));
    print_str(A_LABEL.0);
    print_u32(A_LABEL.1 + HITS.load(Ordering::Relaxed));
    if HITS.fetch_add(1, Ordering::Relaxed) < 1 {
        (ENTRY_TABLE[0])();
    }
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("chunk b");
    print_u32(say(PETS[0]) + say(PETS[1]));
    print_str(B_LABEL.0);
    print_u32(B_LABEL.1);
    print_u32(say(B_SEES_A));
    print_bool(core::ptr::eq(*B_SEES_A, &DOG));
    if HITS.fetch_add(1, Ordering::Relaxed) < 3 {
        (ENTRY_TABLE[0])();
    }
}

#[unsafe(no_mangle)]
pub fn chunk_c() {
    print_str("chunk c");
    let mut t = 0;
    for pet in C_CHAIN.iter() {
        t = t * 10 + say(pet);
    }
    print_u32(t);
    print_u32(say(C_PETS[1]));
    print_bool(core::ptr::eq(*C_CHAIN, &PETS));
    print_u32(HITS.load(Ordering::Relaxed));
    (ENTRY_TABLE[0])();
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
    chunk_c();
}
