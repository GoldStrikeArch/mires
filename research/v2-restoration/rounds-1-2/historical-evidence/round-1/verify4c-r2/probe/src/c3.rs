//! Verifier control: the same shape with a static instead of a constant (no pooling involved).
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

static BASE: [u32; 4] = [10, 11, 12, 13];
/// Read by chunk a only; its initializer points at the static both chunks read.
static INDEX: &[u32; 4] = &BASE;

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_u32(BASE.iter().sum());
    print_usize(INDEX.len());
}

#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_u32(BASE[1]);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
}
