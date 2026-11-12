#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// A `static` named as a chunk's entry point that is a member of a cycle (the admitted refusal).
struct Node { id: u32, next: &'static Node }
#[unsafe(no_mangle)]
pub static ENTRY_C: Node = Node { id: 1, next: &OTHER };
static OTHER: Node = Node { id: 2, next: &ENTRY_C };
#[unsafe(no_mangle)]
pub fn chunk_a() { print_u32(OTHER.next.id + ENTRY_C.next.id * 10); }
#[unsafe(no_mangle)]
pub fn chunk_b() { print_u32(OTHER.next.next.id); }
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); }
