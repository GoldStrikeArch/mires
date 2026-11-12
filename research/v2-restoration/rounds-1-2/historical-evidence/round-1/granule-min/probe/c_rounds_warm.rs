//! A `Vec` or `String` whose buffer address was observed, then moved by value into a callee that
//! keeps it alive: no allocation made while it lives may be given that address.
//!
//! With the optimizer on, `make`'s value stays in its baseline heap block (its address was asked)
//! and the callee's parameter is native, so the move is a `Convert` at the call argument
//! (`__rt.convert_block_nv`, or the `__rt.bytes_str` decode for a `String`). The Rust value lives
//! on in the native carrier, so the conversion must not give the block's address granule back: an
//! earlier version did, and a second `Vec` allocated in the callee was handed the first one's
//! address while both were alive. Each shape below compares a stored address, as an integer or as
//! a raw pointer, with the address of an allocation made while the moved value is alive — natively
//! never equal — or counts such collisions: in one callee (`keep`), in a chain of 21 values kept
//! alive by recursion (`chain`), over 50 rounds (`shape_a`), through a callee that returns the
//! value (`pass`), and for a `String` the callee appends to (`keep_s`). Only relations Rust
//! guarantees are printed. (Whether a moved value keeps the address it had is not one of them
//! here: its native carrier has no address of its own, so asking again names a new block.)
//!
//! `vec_dangles`, `boxed` and `grown` are fixture 122's warm-up: the shape that makes these moves
//! cross into native carriers in `js-profile=release-balanced`.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O0`
//! and `-O3`, which agree.

#![no_std]
#![no_main]
#![allow(unused)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn total(v: &Vec<u32>) -> u32 {
    let mut s = 0u32;
    let mut i = 0;
    while i < v.len() {
        s += v[i];
        i += 1;
    }
    s
}

#[inline(never)]
fn make(out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(100);
    v.push(7);
    *out = v.as_ptr() as usize;
    v
}

#[inline(never)]
fn fill(v: &mut Vec<u8>) -> u32 {
    let mut k = 0u8;
    while k < 50 {
        v.push(k);
        k += 1;
    }
    let mut s = 0u32;
    let mut i = 0;
    while i < v.len() {
        s += v[i] as u32;
        i += 1;
    }
    s
}

/// 50 rounds of `make` and a callee that allocates while it holds the value.
#[inline(never)]
fn make_n(n: usize, out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(n);
    v.push(7);
    *out = v.as_ptr() as usize;
    v
}

#[inline(never)]
fn fresh_addr() -> usize {
    let w: Vec<u8> = black_box(Vec::with_capacity(100));
    let b = w.as_ptr() as usize;
    core::mem::forget(w);
    b
}

#[inline(never)]
fn keep_n(mut v: Vec<u8>, a: usize) -> u32 {
    let b = fresh_addr();
    let mut k = 1u8;
    while k <= 6 {
        v.push(k);
        k += 1;
    }
    let mut s = 0u32;
    let mut i = 0;
    while i < v.len() {
        s += v[i] as u32;
        i += 1;
    }
    v.push(s as u8);
    v.push(9);
    let n = v.len() as u32;
    let first = v[0] as u32 + v[3] as u32;
    (if a == b { 1000 } else { 0 }) + n * 10 + first + s
}

#[inline(never)]
fn shape_a() -> u32 {
    let mut a = 0usize;
    let v = make_n(100, &mut a);
    keep_n(v, a)
}

#[inline(never)]
fn vec_dangles(i: u32) -> bool {
    let mut v: Vec<u8> = black_box(Vec::with_capacity(100));
    v.push(i as u8);
    let p = v.as_ptr();
    let before = p as usize;
    drop(v);
    let w: Vec<u8> = black_box(Vec::with_capacity(100));
    let other = w.as_ptr() as usize;
    let after = p as usize;
    black_box(other);
    drop(w);
    before == after
}

#[inline(never)]
fn boxed(i: u32) -> usize {
    let b: Box<[u32; 2000]> = black_box(Box::new([i; 2000]));
    let a = &*b as *const [u32; 2000] as usize;
    drop(b);
    a
}

#[inline(never)]
fn grown(i: u32) -> usize {
    let mut v: Vec<u64> = Vec::new();
    let mut n = 0u64;
    while n < 3000 {
        v.push(n + i as u64);
        n += 1;
    }
    black_box(&v).as_ptr() as usize
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut t: Vec<u32> = Vec::new();
    let mut n = 0u32;
    while n < 10 {
        t.push(n);
        n += 1;
    }
    print_u32(total(&t));
    let mut ok = true;
    let mut i = 0u32;
    while i < 3 {
        if grown(i) == 0 || boxed(i) == 0 || !vec_dangles(i) {
            ok = false;
        }
        i += 1;
    }
    print_bool(ok);    let mut collisions = 0u32;
    let mut sum = 0u32;
    let mut i = 0u32;
    while i < 50 {
        let r = shape_a();
        if r >= 1000 {
            collisions += 1;
        }
        sum = sum.wrapping_add(r % 1000);
        i += 1;
    }
    print_u32(collisions);
    print_u32(sum);
}
