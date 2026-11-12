#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// Static chunk entries with a reference in ONE direction only:
//  ENTRY_UP names the shared LEAF, and no shared binding names it (functions do);
//  ENTRY_DOWN names nothing shared, and the shared HOLD names it.
struct Leaf { n: u32 }
static LEAF: Leaf = Leaf { n: 3 };
struct Node { id: u32, leaf: &'static Leaf }
#[unsafe(no_mangle)]
pub static ENTRY_UP: Node = Node { id: 1, leaf: &LEAF };
struct Plain { id: u32 }
#[unsafe(no_mangle)]
pub static ENTRY_DOWN: Plain = Plain { id: 7 };
struct Hold { n: &'static Plain }
static HOLD: Hold = Hold { n: &ENTRY_DOWN };
#[unsafe(no_mangle)]
pub fn chunk_a() { print_u32(HOLD.n.id + ENTRY_UP.leaf.n * 10 + ENTRY_UP.id * 100); }
#[unsafe(no_mangle)]
pub fn chunk_b() { print_u32(HOLD.n.id + LEAF.n * 10 + ENTRY_UP.id * 100); print_bool(core::ptr::eq(HOLD.n, &ENTRY_DOWN)); }
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); }
