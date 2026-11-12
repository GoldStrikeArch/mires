#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// A `static` named as a chunk's entry point (js-chunk=s:ENTRY_S), not in any cycle of statics.
// ENTRY_S names the shared LEAF; the shared HOLD names ENTRY_S: the files s.js and shared.js
// import each other, and both imports are read by a `let` initializer at module evaluation.
struct Leaf { n: u32 }
static LEAF: Leaf = Leaf { n: 3 };
struct Node { id: u32, leaf: &'static Leaf }
#[unsafe(no_mangle)]
pub static ENTRY_S: Node = Node { id: 1, leaf: &LEAF };
struct Hold { n: &'static Node }
static HOLD: Hold = Hold { n: &ENTRY_S };
#[unsafe(no_mangle)]
pub fn chunk_a() { print_u32(HOLD.n.id + HOLD.n.leaf.n * 10); }
#[unsafe(no_mangle)]
pub fn chunk_b() { print_u32(HOLD.n.leaf.n + LEAF.n * 10); }
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); }
