//! Safe-Rust `sort_unstable`, `sort`, and `select_nth_unstable` on `String` and owned `Drop`
//! aggregates across smallsort thresholds and distributions (MIRES-022).
//!
//! Exercises `core::slice::sort`'s `small_sort_general` (`MaybeUninit<[T; 48]>` scratch buffer
//! passed via `slice::from_raw_parts_mut` -> `sort8_stable` / `sort4_stable`) as well as
//! `small_sort_fallback` (`size > 85`), checking sortedness, element preservation, and exact
//! `Drop` counts (`created == dropped`, XOR checksum of dropped IDs) with zero `unsafe`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::sync::atomic::{AtomicU32, Ordering as AtomicOrd};

static DROPPED_COUNT: AtomicU32 = AtomicU32::new(0);
static DROPPED_XOR: AtomicU32 = AtomicU32::new(0);

fn reset_drops() {
    DROPPED_COUNT.store(0, AtomicOrd::SeqCst);
    DROPPED_XOR.store(0, AtomicOrd::SeqCst);
}

/// 12-byte `Freeze` + `Drop` aggregate (`size <= 16` -> `small_sort_general` -> `sort8_stable`).
#[derive(Eq, PartialEq)]
struct Tracked12 {
    key: u32,
    id: u32,
    tag: u32,
}

impl Ord for Tracked12 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for Tracked12 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Drop for Tracked12 {
    fn drop(&mut self) {
        DROPPED_COUNT.fetch_add(1, AtomicOrd::SeqCst);
        DROPPED_XOR.fetch_xor(self.id ^ (self.tag << 16), AtomicOrd::SeqCst);
    }
}

/// 24-byte `Freeze` + `Drop` aggregate (`16 < size <= 85` -> `small_sort_general` -> `sort4_stable`).
#[derive(Eq, PartialEq)]
struct Tracked24 {
    key: u32,
    id: u32,
    pad: [u32; 4],
}

impl Ord for Tracked24 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for Tracked24 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Drop for Tracked24 {
    fn drop(&mut self) {
        DROPPED_COUNT.fetch_add(1, AtomicOrd::SeqCst);
        DROPPED_XOR.fetch_xor(self.id ^ self.pad[3], AtomicOrd::SeqCst);
    }
}

/// 96-byte `Freeze` + `Drop` aggregate (`size * 48 > 4096` -> `small_sort_fallback`).
#[derive(Eq, PartialEq)]
struct Tracked96 {
    key: u32,
    id: u32,
    pad: [u32; 22],
}

impl Ord for Tracked96 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for Tracked96 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Drop for Tracked96 {
    fn drop(&mut self) {
        DROPPED_COUNT.fetch_add(1, AtomicOrd::SeqCst);
        DROPPED_XOR.fetch_xor(self.id ^ self.pad[21], AtomicOrd::SeqCst);
    }
}

fn lcg(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    *state
}

fn key_for_dist(dist: usize, i: usize, n: usize, rng: &mut u32) -> u32 {
    match dist {
        0 => i as u32,              // already sorted
        1 => (n - 1 - i) as u32,    // reverse sorted
        2 => 7,                     // all equal
        3 => (i % 5) as u32,        // few unique keys
        4 => ((i * 7) % 13) as u32, // sawtooth
        5 => {
            // pipe-organ
            let mid = n / 2;
            if i < mid {
                i as u32
            } else {
                (n - 1 - i) as u32
            }
        }
        _ => lcg(rng) >> 8, // pseudo-random
    }
}

fn make_word(k: u32) -> String {
    let mut s = String::new();
    s.push(char::from(b'a' + ((k / 676) % 26) as u8));
    s.push(char::from(b'a' + ((k / 26) % 26) as u8));
    s.push(char::from(b'a' + (k % 26) as u8));
    s
}

fn hash_strings(xs: &[String]) -> u32 {
    let mut h: u32 = 2166136261;
    for s in xs {
        for &b in s.as_bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(16777619);
        }
        h ^= 0xff;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn check_sorted_str(xs: &[String]) -> bool {
    xs.windows(2).all(|w| w[0] <= w[1])
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    let lengths = [
        0usize, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 19, 20, 21, 22, 24, 31, 32, 33, 39, 40, 41, 64,
        100, 150, 250,
    ];

    // 1. Vec<String>::sort_unstable across all lengths (pseudo-random).
    let mut rng = 0x1234_5678u32;
    let mut all_ok = true;
    let mut combined_hash = 0u32;
    for &n in &lengths {
        let mut v: Vec<String> = (0..n)
            .map(|i| make_word(key_for_dist(6, i, n, &mut rng)))
            .collect();
        let sum_before: u32 = v
            .iter()
            .map(|s| s.as_bytes().iter().map(|&b| b as u32).sum::<u32>())
            .fold(0u32, u32::wrapping_add);
        v.sort_unstable();
        let sum_after: u32 = v
            .iter()
            .map(|s| s.as_bytes().iter().map(|&b| b as u32).sum::<u32>())
            .fold(0u32, u32::wrapping_add);
        if !check_sorted_str(&v) || sum_before != sum_after {
            all_ok = false;
        }
        combined_hash = combined_hash.wrapping_add(hash_strings(&v));
    }
    print_bool(all_ok);
    print_u32(combined_hash);

    // 2. All 7 distributions at lengths 21, 32, 64 for sort_unstable, sort, sort_unstable_by,
    //    sort_unstable_by_key, sort_by, sort_by_key, select_nth_unstable, select_nth_unstable_by.
    let mut dist_ok = true;
    for &n in &[21usize, 32, 64] {
        for dist in 0..7 {
            let mut r = (n as u32) ^ ((dist as u32) << 12) ^ 0x9e37_79b9;
            let base: Vec<String> = (0..n)
                .map(|i| make_word(key_for_dist(dist, i, n, &mut r)))
                .collect();

            let mut a = base.clone();
            a.sort_unstable();
            let mut b = base.clone();
            b.sort();
            if a != b || !check_sorted_str(&a) {
                dist_ok = false;
            }

            let mut c = base.clone();
            c.sort_unstable_by(|x, y| y.cmp(x));
            let mut d = base.clone();
            d.sort_by(|x, y| y.cmp(x));
            if c != d || !c.windows(2).all(|w| w[0] >= w[1]) {
                dist_ok = false;
            }

            let mut e = base.clone();
            e.sort_unstable_by_key(|s| (s.as_bytes()[2], s.as_bytes()[0], s.as_bytes()[1]));
            let mut f = base.clone();
            f.sort_by_key(|s| (s.as_bytes()[2], s.as_bytes()[0], s.as_bytes()[1]));
            if !e.windows(2).all(|w| {
                (w[0].as_bytes()[2], w[0].as_bytes()[0], w[0].as_bytes()[1])
                    <= (w[1].as_bytes()[2], w[1].as_bytes()[0], w[1].as_bytes()[1])
            }) || !f.windows(2).all(|w| {
                (w[0].as_bytes()[2], w[0].as_bytes()[0], w[0].as_bytes()[1])
                    <= (w[1].as_bytes()[2], w[1].as_bytes()[0], w[1].as_bytes()[1])
            }) {
                dist_ok = false;
            }

            let mid = n / 2;
            let mut g = base.clone();
            let (lo, pivot, hi) = g.select_nth_unstable(mid);
            if *pivot != a[mid] || lo.iter().any(|x| x > pivot) || hi.iter().any(|x| x < pivot) {
                dist_ok = false;
            }

            let mut h = base.clone();
            let (lo2, pivot2, hi2) = h.select_nth_unstable_by(mid, |x, y| y.cmp(x));
            if *pivot2 != c[mid] || lo2.iter().any(|x| x < pivot2) || hi2.iter().any(|x| x > pivot2)
            {
                dist_ok = false;
            }
        }
    }
    print_bool(dist_ok);

    // 3. Owned heap elements: Vec<Vec<u32>>, Vec<Box<u32>>, Vec<Box<String>>.
    let mut owned_ok = true;
    for &n in &[21usize, 32, 50] {
        let mut vv: Vec<Vec<u32>> = (0..n)
            .map(|i| {
                let k = ((i * 17 + 5) % n) as u32;
                alloc::vec![k, k.wrapping_mul(3), i as u32]
            })
            .collect();
        vv.sort_unstable();
        if !vv.windows(2).all(|w| w[0] <= w[1]) || vv.iter().any(|x| x.len() != 3) {
            owned_ok = false;
        }

        let mut vb: Vec<Box<u32>> = (0..n)
            .map(|i| Box::new(((i * 13 + 7) % n) as u32))
            .collect();
        vb.sort_unstable();
        if !vb.windows(2).all(|w| w[0] <= w[1]) {
            owned_ok = false;
        }

        let mut vbs: Vec<Box<String>> = (0..n)
            .map(|i| Box::new(make_word(((i * 37 + 11) % 700) as u32)))
            .collect();
        vbs.sort_unstable();
        if !vbs.windows(2).all(|w| w[0] <= w[1]) {
            owned_ok = false;
        }
    }
    print_bool(owned_ok);

    // 4. Freeze + Drop-counted structs at sizes 12, 24, and 96 bytes (exact drop count & XOR).
    let mut drop_ok = true;
    for &n in &[21usize, 32, 41, 100] {
        let mut expected_xor = 0u32;
        reset_drops();
        {
            let mut v12: Vec<Tracked12> = (0..n)
                .map(|i| {
                    let id = (i as u32) + 1;
                    let tag = ((i * 7 + 3) % 251) as u32;
                    expected_xor ^= id ^ (tag << 16);
                    Tracked12 {
                        key: ((i * 29 + 11) % 19) as u32,
                        id,
                        tag,
                    }
                })
                .collect();
            v12.sort_unstable();
            if !v12.windows(2).all(|w| w[0] <= w[1]) {
                drop_ok = false;
            }
        }
        if DROPPED_COUNT.load(AtomicOrd::SeqCst) != n as u32
            || DROPPED_XOR.load(AtomicOrd::SeqCst) != expected_xor
        {
            drop_ok = false;
        }

        expected_xor = 0;
        reset_drops();
        {
            let mut v24: Vec<Tracked24> = (0..n)
                .map(|i| {
                    let id = (i as u32) + 1000;
                    let tail = (i as u32).wrapping_mul(17);
                    expected_xor ^= id ^ tail;
                    Tracked24 {
                        key: ((i * 31 + 7) % 23) as u32,
                        id,
                        pad: [1, 2, 3, tail],
                    }
                })
                .collect();
            v24.sort_unstable();
            if !v24.windows(2).all(|w| w[0] <= w[1]) {
                drop_ok = false;
            }
        }
        if DROPPED_COUNT.load(AtomicOrd::SeqCst) != n as u32
            || DROPPED_XOR.load(AtomicOrd::SeqCst) != expected_xor
        {
            drop_ok = false;
        }

        expected_xor = 0;
        reset_drops();
        {
            let mut v96: Vec<Tracked96> = (0..n)
                .map(|i| {
                    let id = (i as u32) + 2000;
                    let tail = (i as u32).wrapping_mul(19);
                    expected_xor ^= id ^ tail;
                    let mut pad = [0u32; 22];
                    pad[21] = tail;
                    Tracked96 {
                        key: ((i * 19 + 5) % 17) as u32,
                        id,
                        pad,
                    }
                })
                .collect();
            v96.sort_unstable();
            if !v96.windows(2).all(|w| w[0] <= w[1]) {
                drop_ok = false;
            }
        }
        if DROPPED_COUNT.load(AtomicOrd::SeqCst) != n as u32
            || DROPPED_XOR.load(AtomicOrd::SeqCst) != expected_xor
        {
            drop_ok = false;
        }
    }
    print_bool(drop_ok);
}
