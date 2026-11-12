#![no_std]
#![no_main]
#![allow(dead_code, unused, private_interfaces)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// A `static` entry point that names a shared BINDING (LEAF), while no shared binding names it:
// only the shared FUNCTION `helper` does. CONTRACT.md says such an entry "may name shared bindings
// when no shared binding names it"; the rule (rightly) refuses it, because shared.js still imports
// s.js back, so loading any file but s.js first evaluates s.js before shared.js.
struct Leaf { n: u32 }
static LEAF: Leaf = Leaf { n: 3 };
struct Node { id: u32, leaf: &'static Leaf }
#[unsafe(no_mangle)]
pub static ENTRY_S: Node = Node { id: 1, leaf: &LEAF };
#[inline(never)]
fn helper() -> u32 { ENTRY_S.id + ENTRY_S.leaf.n * 10 }
#[unsafe(no_mangle)]
pub fn chunk_a() { print_u32(helper() + LEAF.n * 100); }
#[unsafe(no_mangle)]
pub fn chunk_b() { print_u32(helper() + LEAF.n * 1000); }
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); }
