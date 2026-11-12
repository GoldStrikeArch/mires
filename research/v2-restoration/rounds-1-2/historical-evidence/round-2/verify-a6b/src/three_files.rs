#![no_std]
#![no_main]
#![allow(dead_code, unused, private_interfaces)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};

// A cycle of imports over THREE files where the file that is read from does not import the
// reader directly. Split `s:ENTRY_S a:chunk_a b:chunk_b`:
//   s.js      ENTRY_S, whose initializer reads LEAF            imports shared.js (a binding read)
//   shared.js LEAF, helper (reached by a and b)                imports a.js (the function chunk_a)
//   a.js      chunk_a, which names ENTRY_S in its body         imports s.js and shared.js
// shared.js -> a.js -> s.js -> shared.js. Loading shared.js or b.js first evaluates s.js before
// shared.js, and `let ENTRY_S = { .., leaf: LEAF }` reads LEAF uninitialized.
static DEPTH: AtomicU32 = AtomicU32::new(0);
struct Leaf { n: u32 }
static LEAF: Leaf = Leaf { n: 3 };
struct Node { id: u32, leaf: &'static Leaf }
#[unsafe(no_mangle)]
pub static ENTRY_S: Node = Node { id: 1, leaf: &LEAF };
#[inline(never)]
fn helper() -> u32 {
    if DEPTH.fetch_add(1, Ordering::Relaxed) == 1 { chunk_a(); }
    LEAF.n
}
#[unsafe(no_mangle)]
pub fn chunk_a() { print_u32(ENTRY_S.id + ENTRY_S.leaf.n * 10 + helper() * 100); }
#[unsafe(no_mangle)]
pub fn chunk_b() { print_u32(helper() * 1000); }
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); }
