//! A `static` may be a chunk's entry point when its imports run one way.
//!
//! Split four ways (`265_chunk_static_entries.chunks`, run by `scripts/chunk-test.sh`), two of the
//! entry points are statics, which a chunk exports rather than calls. `ENTRY_UP` names the shared
//! `LEAF`, so `u.js` imports from the shared chunk, and only functions name `ENTRY_UP`.
//! `ENTRY_DOWN` names nothing, and the shared `HOLD` names it, so the shared chunk imports from
//! `d.js`. Neither import comes back, so every file is evaluated after the files it imports, in
//! whichever order a host loads them. (When the imports do come back and an initializer reads one,
//! the split is refused: `266_chunk_static_entry_refusals`.) The program also runs unsplit.
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
/// An entry point naming a binding the shared chunk owns.
#[unsafe(no_mangle)]
pub static ENTRY_UP: Node = Node { id: 1, leaf: &LEAF };

struct Plain {
    id: u32,
}
/// An entry point the shared chunk names.
#[unsafe(no_mangle)]
pub static ENTRY_DOWN: Plain = Plain { id: 7 };

struct Hold {
    n: &'static Plain,
}
static HOLD: Hold = Hold { n: &ENTRY_DOWN };

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_u32(HOLD.n.id + ENTRY_UP.leaf.n * 10 + ENTRY_UP.id * 100);
    print_bool(core::ptr::eq(ENTRY_UP.leaf, &LEAF));
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_u32(HOLD.n.id + LEAF.n * 10 + ENTRY_UP.id * 100);
    print_bool(core::ptr::eq(HOLD.n, &ENTRY_DOWN));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
}
