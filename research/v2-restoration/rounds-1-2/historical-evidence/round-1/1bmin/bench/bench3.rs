//! Micro-benchmark: element moves (`slice::swap`, `reverse`, `sort_by_key`, `ptr::copy`,
//! `copy_from_slice`) of aggregates held by value, of references, and of integers.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;
use prelude::*;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}
#[derive(Clone, Copy)]
struct S {
    p: P,
    z: u32,
}

fn mkv(n: u32) -> Vec<S> {
    let mut v = Vec::new();
    for i in 0..n {
        v.push(S { p: P { x: i.wrapping_mul(2654435761) >> 7, y: i }, z: i });
    }
    v
}

/// Aggregates by value: swaps, reverses and a sort, all through `ptr::copy*` with a clone.
#[unsafe(no_mangle)]
pub fn b3_value_moves(n: u32) -> u32 {
    let mut v = black_box(mkv(64));
    let mut acc = 0u32;
    for i in 0..n {
        let a = (i & 63) as usize;
        let b = (i.wrapping_mul(7) & 63) as usize;
        v.swap(a, b);
        if i & 255 == 0 {
            v.reverse();
        }
        if i & 4095 == 0 {
            v.sort_by_key(|s| s.p.x ^ (i & 15));
        }
        acc = acc.wrapping_add(v[a].p.x ^ v[b].z);
    }
    acc
}

/// References: the same moves on `&S`, which rebind.
#[unsafe(no_mangle)]
pub fn b3_ref_moves(n: u32) -> u32 {
    let backing = black_box(mkv(64));
    let mut v: Vec<&S> = backing.iter().collect();
    let mut acc = 0u32;
    for i in 0..n {
        let a = (i & 63) as usize;
        let b = (i.wrapping_mul(7) & 63) as usize;
        v.swap(a, b);
        if i & 255 == 0 {
            v.reverse();
        }
        if i & 4095 == 0 {
            v.sort_by_key(|s| s.p.x ^ (i & 15));
        }
        acc = acc.wrapping_add(v[a].p.x ^ v[b].z);
    }
    acc
}

/// Integers: swaps and a sort of `u32`.
#[unsafe(no_mangle)]
pub fn b3_int_moves(n: u32) -> u32 {
    let mut v: Vec<u32> = black_box((0..64u32).map(|i| i.wrapping_mul(2654435761)).collect());
    let mut acc = 0u32;
    for i in 0..n {
        let a = (i & 63) as usize;
        let b = (i.wrapping_mul(7) & 63) as usize;
        v.swap(a, b);
        if i & 4095 == 0 {
            v.sort_by_key(|x| *x ^ (i & 15));
        }
        acc = acc.wrapping_add(v[a] ^ v[b]);
    }
    acc
}

/// Bulk moves between two buffers: `copy_from_slice` and an overlapping `ptr::copy`.
#[unsafe(no_mangle)]
pub fn b3_bulk(n: u32) -> u32 {
    let src = black_box(mkv(32));
    let mut dst = black_box(mkv(32));
    let mut ints: Vec<u32> = black_box((0..64u32).collect());
    let mut acc = 0u32;
    for i in 0..n / 16 {
        dst.copy_from_slice(&src);
        let p = ints.as_mut_ptr();
        unsafe { ptr::copy(p, p.add(1), 32) };
        acc = acc.wrapping_add(dst[(i & 31) as usize].p.x ^ ints[(i & 31) as usize]);
    }
    acc
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(b3_value_moves(10000));
    print_u32(b3_ref_moves(10000));
    print_u32(b3_int_moves(10000));
    print_u32(b3_bulk(10000));
}
