//! Safe-Rust `sort_unstable`, `sort`, and `select_nth_unstable` across Copy aggregates,
//! primitives (`u8`, `i32`, `u64`, `i128`, `&str`), and ZSTs (MIRES-022).
//!
//! Covers both `small_sort_network` (`Copy`, `size <= 8`: `u8`, `i32`, `u64`, `(u32, u32)`, `&str`)
//! and `small_sort_general` (`Copy`, `size > 8`: `i128`, 16-byte/24-byte `Copy` structs), plus ZSTs.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct Rec16 {
    k: u32,
    a: u64,
    b: u32,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct Rec24 {
    k: u32,
    a: u64,
    b: u64,
    c: u32,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct Z;

fn lcg(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    *state
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    let lengths = [0usize, 1, 2, 8, 16, 17, 20, 21, 24, 31, 32, 33, 40, 64, 150];

    // 1. Large Copy structs (size > 8 -> small_sort_general: sort8_stable & sort4_stable).
    let mut rec_ok = true;
    let mut rec_checksum = 0u32;
    let mut rng = 0x55aa_1357u32;
    for &n in &lengths {
        let mut r16: Vec<Rec16> = (0..n)
            .map(|i| Rec16 {
                k: lcg(&mut rng) % 29,
                a: (lcg(&mut rng) as u64) << 16 | (i as u64),
                b: i as u32,
            })
            .collect();
        let mut r16_stable = r16.clone();
        r16.sort_unstable();
        r16_stable.sort();
        if r16 != r16_stable || !r16.windows(2).all(|w| w[0] <= w[1]) {
            rec_ok = false;
        }
        if let Some(last) = r16.last() {
            rec_checksum = rec_checksum.wrapping_add(last.k ^ last.b);
        }

        let mut r24: Vec<Rec24> = (0..n)
            .map(|i| Rec24 {
                k: lcg(&mut rng) % 31,
                a: lcg(&mut rng) as u64,
                b: (i as u64).wrapping_mul(13),
                c: i as u32,
            })
            .collect();
        let mut r24_stable = r24.clone();
        r24.sort_unstable();
        r24_stable.sort();
        if r24 != r24_stable || !r24.windows(2).all(|w| w[0] <= w[1]) {
            rec_ok = false;
        }
        if n > 0 {
            let mut r24_sel = r24_stable.clone();
            r24_sel.reverse();
            let mid = n / 2;
            let (lo, piv, hi) = r24_sel.select_nth_unstable(mid);
            if *piv != r24[mid] || lo.iter().any(|x| x > piv) || hi.iter().any(|x| x < piv) {
                rec_ok = false;
            }
        }
    }
    print_bool(rec_ok);
    print_u32(rec_checksum);

    // 2. Small Copy tuple `(u32, u32)` (size 8 -> small_sort_network).
    let mut pair_ok = true;
    for &n in &lengths {
        let mut vp: Vec<(u32, u32)> = (0..n)
            .map(|i| (lcg(&mut rng) % 19, (n - i) as u32))
            .collect();
        let mut vp_s = vp.clone();
        vp.sort_unstable();
        vp_s.sort();
        if vp != vp_s || !vp.windows(2).all(|w| w[0] <= w[1]) {
            pair_ok = false;
        }
    }
    print_bool(pair_ok);

    // 3. Primitives: u8, i32, u64, i128 (size 16 -> small_sort_general!), &str.
    let mut prim_ok = true;
    let words: [&str; 12] = [
        "pear", "apple", "orange", "banana", "kiwi", "plum", "fig", "grape", "mango", "lime",
        "peach", "cherry",
    ];
    for &n in &lengths {
        let mut vu8: Vec<u8> = (0..n).map(|_| (lcg(&mut rng) >> 24) as u8).collect();
        let mut vu8_s = vu8.clone();
        vu8.sort_unstable();
        vu8_s.sort();
        if vu8 != vu8_s {
            prim_ok = false;
        }

        let mut vi32: Vec<i32> = (0..n).map(|_| lcg(&mut rng) as i32).collect();
        let mut vi32_s = vi32.clone();
        vi32.sort_unstable();
        vi32_s.sort();
        if vi32 != vi32_s {
            prim_ok = false;
        }

        let mut vu64: Vec<u64> = (0..n)
            .map(|_| ((lcg(&mut rng) as u64) << 32) | (lcg(&mut rng) as u64))
            .collect();
        let mut vu64_s = vu64.clone();
        vu64.sort_unstable();
        vu64_s.sort();
        if vu64 != vu64_s {
            prim_ok = false;
        }

        let mut vi128: Vec<i128> = (0..n)
            .map(|_| {
                let hi = (lcg(&mut rng) as i32) as i128;
                let lo = lcg(&mut rng) as u32 as i128;
                (hi << 64) ^ (lo << 32) ^ (lcg(&mut rng) as i128)
            })
            .collect();
        let mut vi128_s = vi128.clone();
        vi128.sort_unstable();
        vi128_s.sort();
        if vi128 != vi128_s {
            prim_ok = false;
        }

        let mut vs: Vec<&str> = (0..n).map(|i| words[(i * 7 + 3) % words.len()]).collect();
        let mut vs_s = vs.clone();
        vs.sort_unstable();
        vs_s.sort();
        if vs != vs_s {
            prim_ok = false;
        }
    }
    print_bool(prim_ok);

    // 4. ZSTs: `()` and `struct Z`.
    let mut zst_ok = true;
    for &n in &[0usize, 1, 21, 32, 64] {
        let mut vu: Vec<()> = alloc::vec![(); n];
        vu.sort_unstable();
        vu.sort();
        if vu.len() != n {
            zst_ok = false;
        }
        let mut vz: Vec<Z> = alloc::vec![Z; n];
        vz.sort_unstable();
        vz.sort();
        if vz.len() != n {
            zst_ok = false;
        }
        if n > 0 {
            let (lo, _, hi) = vz.select_nth_unstable(n / 2);
            if lo.len() + 1 + hi.len() != n {
                zst_ok = false;
            }
        }
    }
    print_bool(zst_ok);
}
