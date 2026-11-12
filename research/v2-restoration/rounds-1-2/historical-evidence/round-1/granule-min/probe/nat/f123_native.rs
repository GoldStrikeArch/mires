//! A `Vec` or `String` whose buffer address was observed, then moved by value into a callee that
//! keeps it alive: no allocation made while it lives may be given that address.
//!
//! With `js-profile=release-balanced`, `make`'s value stays in its baseline heap block (its
//! address was asked) and the callee's parameter is native, so each move below is a `Convert` at
//! the call argument (`__rt.convert_block_nv`, or the `__rt.bytes_str` decode into a builder for
//! the `String`). The Rust value lives on in the native carrier, so that conversion must not give
//! the block's address granule back: an earlier version did, and an allocation made in the callee
//! was handed the moved value's address while both were alive. Each shape compares a stored
//! address, as an integer or as a raw pointer, with the address of an allocation made while the
//! moved value is alive — natively never equal — or counts such collisions: one callee (`keep`), a
//! chain of 21 values kept alive by recursion (`chain`), a callee that hands the value back
//! (`pass`), a raw pointer (`keep_p`), 50 rounds (`shape_a`), and a `String` the callee appends to
//! (`keep_s`). The drops at the end of each callee are cuts in front of a drop, and do give the
//! granule back. Only relations Rust guarantees are printed — whether a moved value keeps its old
//! address is not printed, because its native carrier has no address of its own and asking again
//! names a new block. The six shapes are the ones that kept their call-argument conversions
//! together; adding fixture 122's warm-up loop turns them all off.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O0`
//! and `-O3`, which agree.



extern crate alloc;
use alloc::string::String;
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

/// A second allocation, made while `v` (buffer address `a`) is alive.
#[inline(never)]
fn keep(mut v: Vec<u8>, a: usize) -> bool {
    let w: Vec<u8> = Vec::with_capacity(100);
    let b = w.as_ptr() as usize;
    print_u32(fill(&mut v));
    print_u32(w.capacity() as u32);
    a == b
}

/// 21 values alive at once down the recursion, each compared with the one above it.
#[inline(never)]
fn chain(depth: u32, mut v: Vec<u8>, a: usize, prev: usize) -> u32 {
    let mut k = 0u8;
    while k < 50 {
        v.push(k);
        k += 1;
    }
    let mut a2 = 0usize;
    let below = if depth > 0 {
        let w = make(&mut a2);
        chain(depth - 1, w, a2, a)
    } else {
        0
    };
    let mut s = 0u32;
    let mut i = 0;
    while i < v.len() {
        s += v[i] as u32;
        i += 1;
    }
    below + (if a == prev { 1 } else { 0 }) + (s & 0)
}

/// The callee hands the value back; it is still the same allocation.
#[inline(never)]
fn pass(mut v: Vec<u8>) -> Vec<u8> {
    print_u32(fill(&mut v));
    v
}

/// The address kept as a raw pointer.
#[inline(never)]
fn make_p(out: &mut *const u8) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(100);
    v.push(7);
    *out = v.as_ptr();
    v
}

#[inline(never)]
fn keep_p(mut v: Vec<u8>, p: *const u8) -> bool {
    let w: Vec<u8> = Vec::with_capacity(100);
    let q = w.as_ptr();
    print_u32(fill(&mut v) + w.capacity() as u32);
    (p as usize) == (q as usize)
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

/// A `String` the callee appends to.
#[inline(never)]
fn make_s(out: &mut usize) -> String {
    let s: String = black_box(String::with_capacity(100));
    *out = s.as_bytes().as_ptr() as usize;
    s
}

#[inline(never)]
fn other() -> usize {
    let w: Vec<u8> = Vec::with_capacity(100);
    let b = w.as_ptr() as usize;
    core::mem::forget(w);
    b
}

#[inline(never)]
fn keep_s(mut s: String, a: usize) -> bool {
    let b = other();
    let mut k = 0u32;
    while k < 20 {
        s.push_str("xy");
        s.push('z');
        s.push_str("--");
        k += 1;
    }
    s.push_str("end");
    s.push('!');
    print_str(&s);
    a == b
}

#[inline(never)]
fn build(n: u32) -> String {
    let mut t = String::new();
    let mut k = 0u32;
    while k < n {
        t.push_str("ab");
        t.push('c');
        k += 1;
    }
    t
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

    let mut a = 0usize;
    let v = make(&mut a);
    print_bool(keep(v, a));

    let mut a = 0usize;
    let v = make(&mut a);
    print_u32(chain(20, v, a, 0));

    let mut a = 0usize;
    let v = make(&mut a);
    let v2 = pass(v);
    let w: Vec<u8> = Vec::with_capacity(100);
    let b = w.as_ptr() as usize;
    print_u32(v2.len() as u32 + w.capacity() as u32);
    print_bool(a == b);

    let mut p: *const u8 = core::ptr::null();
    let v = make_p(&mut p);
    print_bool(keep_p(v, p));

    let mut collisions = 0u32;
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

    print_str(&build(3));
    let mut a = 0usize;
    let s = make_s(&mut a);
    print_bool(keep_s(s, a));
}
#[allow(dead_code)] fn print_bool(x: bool) { println!("{}", x) }
#[allow(dead_code)] fn print_u32(x: u32) { println!("{}", x) }
#[allow(dead_code)] fn print_usize(x: usize) { println!("{}", x) }
#[allow(dead_code)] fn print_str(x: &str) { println!("{}", x) }
fn main() { rust_entry() }
