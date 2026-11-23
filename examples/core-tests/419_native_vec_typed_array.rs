//! Growable ES2018 TypedArray and plain-Array `NativeVec` operations: construction, growth,
//! insertion, removal, range indexing, iteration, cloning, slice conversion, `mem::replace`/
//! `mem::swap`/`Option::take`/`Result` propagation, `-0.0` preservation, and nested `Vec<Vec<T>>`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem;

struct Bag {
    tag: i32,
    bytes: Vec<u8>,
}

#[inline(never)]
fn make_bag(tag: i32, seed: u8) -> Result<Bag, i32> {
    if tag < 0 {
        return Err(tag);
    }
    let mut bytes = Vec::with_capacity(4);
    bytes.push(seed);
    bytes.push(seed.wrapping_add(1));
    bytes.extend_from_slice(black_box(&[seed.wrapping_add(2), seed.wrapping_add(3)]));
    Ok(Bag { tag, bytes })
}

#[inline(never)]
fn use_bag(tag: i32, seed: u8) -> Result<i32, i32> {
    let mut bag = make_bag(tag, seed)?;
    bag.bytes.push(99);
    let mut sum = bag.tag;
    for b in &bag.bytes {
        sum += *b as i32;
    }
    Ok(sum)
}

#[inline(never)]
fn take_opt_vec(opt: &mut Option<Vec<u16>>) -> usize {
    match opt.take() {
        Some(mut v) => {
            v.push(777);
            let mut sum = 0usize;
            for x in &v {
                sum += *x as usize;
            }
            sum
        }
        None => 0,
    }
}

#[inline(never)]
fn test_typed_array_ops() {
    // u8: new, default, push, insert, reserve, reserve_exact, shrink_to_fit, pop, resize, remove, swap_remove
    let mut a: Vec<u8> = Vec::default();
    print_bool(a.is_empty());
    a.reserve(4);
    a.push(10);
    a.push(30);
    a.insert(1, 20);
    a.insert(3, 40);
    print_usize(a.len());
    print_u32(a[0] as u32);
    print_u32(a[1] as u32);
    print_u32(a[2] as u32);
    print_u32(a[3] as u32);
    print_u32(a.remove(1) as u32);
    print_u32(a.swap_remove(0) as u32);
    print_usize(a.len());
    print_u32(a[0] as u32);
    print_u32(a[1] as u32);

    a.reserve_exact(12);
    print_bool(a.capacity() >= 14);
    a.resize(5, 7);
    a.truncate(4);
    a.shrink_to_fit();
    print_usize(a.len());
    print_usize(a.capacity());
    print_u32(a.pop().unwrap_or(0) as u32);
    print_u32(a.pop().unwrap_or(0) as u32);
    print_u32(a.pop().unwrap_or(0) as u32);
    print_u32(a.pop().unwrap_or(0) as u32);
    print_bool(a.pop().is_none());

    // Range indexing (.., a..b, a.., ..b) shared + mutable + extend_from_slice + clone + to_vec
    let mut b: Vec<u32> = vec![10, 20, 30, 40, 50];
    b[1..4][1] = 300;
    b[3..][0] = 400;
    b[..2][0] = 100;
    b[..][4] = 500;
    let mut c = b.clone();
    c.extend_from_slice(&b[1..4]);
    let d = b[2..5].to_vec();
    let mut sum_c = 0u32;
    for x in &c {
        sum_c += *x;
    }
    print_u32(sum_c);
    let mut sum_d = 0u32;
    for x in &d {
        sum_d += *x;
    }
    print_u32(sum_d);

    // Owned extend + mem::replace + mem::swap + iter_mut
    let mut e: Vec<i32> = vec![-1, -2];
    let f: Vec<i32> = vec![3, 4, 5];
    e.extend(f);
    for x in &mut e {
        *x *= 2;
    }
    let mut g: Vec<i32> = vec![100];
    let old = mem::replace(&mut e, vec![9, 8]);
    mem::swap(&mut e, &mut g);
    print_i32(old[0] + old[4]);
    print_i32(e[0]);
    print_i32(g[0] + g[1]);

    // i8, i16, u16, f32, f64 (including -0.0 preservation in from_elem)
    let vi8: Vec<i8> = vec![-128, 127];
    let vi16: Vec<i16> = vec![-1000, 1000];
    let vu16: Vec<u16> = vec![65535, 1];
    let vf32: Vec<f32> = vec![-0.0f32; 3];
    let vf64: Vec<f64> = vec![-0.0f64; 3];
    print_i32(vi8[0] as i32 + vi8[1] as i32);
    print_i32(vi16[0] as i32 + vi16[1] as i32);
    print_u32(vu16[0] as u32 + vu16[1] as u32);
    print_bool(vf32[2].to_bits() == (-0.0f32).to_bits());
    print_bool(vf64[2].to_bits() == (-0.0f64).to_bits());
}

#[inline(never)]
fn test_plain_array_and_nested() {
    // Plain-Array elements: bool, char, u64, String, and nested Vec<Vec<u16>>
    let mut flags: Vec<bool> = vec![true, false, true];
    flags.push(false);
    flags.insert(1, true);
    print_usize(flags.len());
    print_bool(flags.remove(2));

    let mut chars: Vec<char> = vec!['a', 'b', 'c'];
    chars.swap_remove(0);
    print_bool(chars[0] == 'c' && chars[1] == 'b');

    let mut bigs: Vec<u64> = vec![1u64 << 40; 3];
    bigs.push(5);
    print_u64(bigs[0] + bigs[3]);

    let mut words: Vec<String> = Vec::new();
    words.push(String::from("alpha"));
    words.push(String::from("beta"));
    words.push(String::from("gamma"));
    words.truncate(2);
    print_usize(words.len());
    print_str(&words[1]);
    words.clear();
    print_bool(words.is_empty());

    // Nested Vec<Vec<u16>> + drop + Option::take + Result<Bag, i32>
    let mut nested: Vec<Vec<u16>> = Vec::new();
    nested.push(vec![1, 2, 3]);
    nested.push(vec![10, 20]);
    nested.push(vec![100]);
    nested.truncate(2);
    print_usize(nested.len());
    print_u32((nested[0][2] + nested[1][1]) as u32);

    let mut opt = Some(vec![10u16, 20, 30]);
    print_usize(take_opt_vec(&mut opt));
    print_usize(take_opt_vec(&mut opt));

    print_i32(use_bag(5, 10).unwrap_or(-1));
    print_i32(use_bag(-7, 10).unwrap_or(-99));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    test_typed_array_ops();
    test_plain_array_and_nested();
}
