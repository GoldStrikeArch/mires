//! Reconstructed from the complete dev12 source read retained in the conversation.
//! Comments/formatting are new; the test operations are retained. See PROVENANCE.json.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec::Vec;
use core::hint::black_box;
use prelude::*;

struct Z;
impl Drop for Z {
    fn drop(&mut self) { print_str("z"); }
}
#[inline(never)]
fn make() -> [Z; 3] { [Z, Z, Z] }
#[inline(never)]
fn take(a: [Z; 2]) {
    print_usize(black_box(&a).len());
    drop(a);
    print_str("taken");
}
#[inline(never)]
fn look(a: &[Z; 2]) -> usize { a.len() + a.iter().count() }
#[inline(never)]
fn pair() -> [Z; 2] { [Z, Z] }
#[inline(never)]
fn small() -> [[u8; 0]; 3] { [[]; 3] }
#[inline(never)]
fn big() -> [[u8; 0]; 2000] { [[]; 2000] }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let zv: Vec<Z> = alloc::vec![Z, Z, Z];
    print_usize(zv.len());
    drop(zv);
    print_str("vec");
    let mut pushed: Vec<Z> = Vec::new();
    pushed.push(Z);
    pushed.push(Z);
    print_usize(pushed.len());
    drop(pushed);
    print_str("pushed");
    {
        let zs = [Z, Z];
        print_usize(black_box(&zs).len());
    }
    print_str("local");
    let arr = make();
    print_usize(black_box(arr.len()));
    drop(arr);
    print_str("returned");
    take(pair());
    let c = black_box(pair());
    print_usize(look(black_box(&c)));
    drop(c);
    print_str("stored");
    let s = black_box(small());
    print_usize(s.len() + s[black_box(2)].len());
    let b = black_box(big());
    print_usize(b.len() + b[black_box(1999)].len());
    print_usize(b.iter().count());
}
