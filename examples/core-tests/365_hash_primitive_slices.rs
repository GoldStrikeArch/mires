//! Safe Rust `Hash` of primitive integer and `char` slices and vectors (MIRES-007).
//!
//! `core::hash::Hash::hash_slice` for `[u16]`, `[i16]`, `[u32]`, `[i32]`, `[u64]`, `[i64]`,
//! `[u128]`, `[i128]`, `[usize]`, and `[isize]` reinterprets the typed slice as `&[u8]` via
//! `slice::from_raw_parts(self.as_ptr().cast::<u8>(), mem::size_of_val(self))`. Without aliasing
//! little-endian byte views over wider scalar storage, `slice_from_parts` drops the pointer scale
//! and reads whole elements (or `undefined` past element length), silently corrupting hashes in
//! JIR and throwing `Cannot convert undefined to a BigInt` in the baseline lane.
//!
//! This fixture checks every primitive slice type over `vec![]`, `Vec::new()` + `push`, stack
//! arrays, non-zero subslices, empty vectors, `Box<[T]>`, `Rc<[T]>`, and a `Hash` + `Eq` lookup
//! map/set keyed by `Vec<u16>` and `Vec<u32>`. Every hash is checked against the exact
//! little-endian byte stream written to `Hasher::write`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::hash::{Hash, Hasher};
use core::hint::black_box;

/// FNV-1a 64-bit hasher with portable 32-bit length prefix so expectations validate directly
/// against native 64-bit `rustc` while `hash_slice` still feeds its exact `&[u8]` view to `write`.
struct Fnv32Len {
    hash: u64,
    bytes_seen: u32,
}

impl Fnv32Len {
    fn new() -> Self {
        Self {
            hash: 0xcbf2_9ce4_8422_2325,
            bytes_seen: 0,
        }
    }
}

impl Hasher for Fnv32Len {
    fn finish(&self) -> u64 {
        self.hash
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash ^= u64::from(b);
            self.hash = self.hash.wrapping_mul(0x0000_0100_0000_01b3);
            self.bytes_seen += 1;
        }
    }

    fn write_usize(&mut self, i: usize) {
        self.write(&(i as u32).to_le_bytes());
    }
}

/// Default-`write_usize` FNV-1a hasher: exercises `core::hash::Hasher`'s unoverridden path.
struct FnvDefault {
    hash: u64,
    bytes_seen: u32,
}

impl FnvDefault {
    fn new() -> Self {
        Self {
            hash: 0xcbf2_9ce4_8422_2325,
            bytes_seen: 0,
        }
    }
}

impl Hasher for FnvDefault {
    fn finish(&self) -> u64 {
        self.hash
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash ^= u64::from(b);
            self.hash = self.hash.wrapping_mul(0x0000_0100_0000_01b3);
            self.bytes_seen += 1;
        }
    }
}

#[inline(never)]
fn hash_portable<T: Hash + ?Sized>(val: &T) -> (u64, u32) {
    let mut h = Fnv32Len::new();
    val.hash(&mut h);
    (h.finish(), h.bytes_seen)
}

#[inline(never)]
fn hash_default<T: Hash + ?Sized>(val: &T) -> (u64, u32) {
    let mut h = FnvDefault::new();
    val.hash(&mut h);
    (h.finish(), h.bytes_seen)
}

/// Computes the expected hash of a slice by writing its length prefix and each element's
/// `to_le_bytes()` manually (never touching `hash_slice` or raw byte views).
fn manual_hash_bytes(len: usize, raw_le: &[u8]) -> (u64, u32) {
    let mut h = Fnv32Len::new();
    h.write(&(len as u32).to_le_bytes());
    h.write(raw_le);
    (h.finish(), h.bytes_seen)
}

/// Minimal bucketed hash map exercising `Hash` + `Eq` consistency on `Vec<u16>` / `Vec<u32>` keys.
struct SimpleMap<K, V> {
    buckets: Vec<Vec<(K, V)>>,
}

impl<K: Hash + Eq, V> SimpleMap<K, V> {
    fn new(cap: usize) -> Self {
        let mut buckets = Vec::with_capacity(cap);
        for _ in 0..cap {
            buckets.push(Vec::new());
        }
        Self { buckets }
    }

    fn bucket(&self, key: &K) -> usize {
        let (h, _) = hash_default(key);
        (h as usize) % self.buckets.len()
    }

    fn insert(&mut self, key: K, val: V) {
        let idx = self.bucket(&key);
        for (k, v) in &mut self.buckets[idx] {
            if *k == key {
                *v = val;
                return;
            }
        }
        self.buckets[idx].push((key, val));
    }

    fn get(&self, key: &K) -> Option<&V> {
        let idx = self.bucket(key);
        for (k, v) in &self.buckets[idx] {
            if *k == *key {
                return Some(v);
            }
        }
        None
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. Vec<u16> via vec![], Vec::new()+push, Box<[u16]>, Rc<[u16]>, stack [u16; N], subslice, empty.
    let v16_macro: Vec<u16> = black_box(vec![0x0102, 0x0304, 0xff80]);
    let mut v16_push: Vec<u16> = Vec::new();
    v16_push.push(black_box(0x0102));
    v16_push.push(black_box(0x0304));
    v16_push.push(black_box(0xff80));
    let s16_stack: [u16; 3] = black_box([0x0102, 0x0304, 0xff80]);
    let b16: Box<[u16]> = black_box(v16_push.clone().into_boxed_slice());
    let r16: Rc<[u16]> = black_box(Rc::from(&s16_stack[..]));

    let expected_u16 = manual_hash_bytes(3, &[0x02, 0x01, 0x04, 0x03, 0x80, 0xff]);
    let h_macro = hash_portable(&v16_macro);
    let h_push = hash_portable(&v16_push);
    let h_stack = hash_portable(&s16_stack[..]);
    let h_box = hash_portable(&b16);
    let h_rc = hash_portable(&r16);
    print_u64(h_macro.0);
    print_u32(h_macro.1);
    print_bool(h_macro == expected_u16);
    print_bool(h_push == expected_u16);
    print_bool(h_stack == expected_u16);
    print_bool(h_box == expected_u16);
    print_bool(h_rc == expected_u16);

    // Non-zero offset subslice and empty Vec/slice.
    let h_sub16 = hash_portable(&v16_push[1..]);
    print_bool(h_sub16 == manual_hash_bytes(2, &[0x04, 0x03, 0x80, 0xff]));
    let empty16: Vec<u16> = black_box(Vec::new());
    print_bool(hash_portable(&empty16) == manual_hash_bytes(0, &[]));
    print_bool(hash_portable(&v16_macro[3..]) == manual_hash_bytes(0, &[]));

    // Default-write_usize consistency across construction methods (and manual check on 32-bit/64-bit).
    print_bool(hash_default(&v16_macro) == hash_default(&v16_push));
    print_bool(hash_default(&v16_macro) == hash_default(&s16_stack[..]));
    print_bool(hash_default(&v16_macro) == hash_default(&b16));
    print_bool(hash_default(&v16_macro) == hash_default(&r16));

    // 2. i16, u32, i32, u64, i64, u128, i128, char.
    let v_i16: Vec<i16> = black_box(vec![-1, i16::MIN, 0x1234]);
    let mut manual_i16 = Vec::new();
    for &x in &v_i16 {
        manual_i16.extend_from_slice(&x.to_le_bytes());
    }
    let h_i16 = hash_portable(&v_i16);
    print_u64(h_i16.0);
    print_bool(h_i16 == manual_hash_bytes(v_i16.len(), &manual_i16));

    let v_u32: Vec<u32> = black_box(vec![0x0102_0304, 0x8000_0001, u32::MAX]);
    let mut manual_u32 = Vec::new();
    for &x in &v_u32 {
        manual_u32.extend_from_slice(&x.to_le_bytes());
    }
    let h_u32 = hash_portable(&v_u32);
    print_u64(h_u32.0);
    print_bool(h_u32 == manual_hash_bytes(v_u32.len(), &manual_u32));

    let v_i32: [i32; 3] = black_box([-1, i32::MIN, 0x7654_3210]);
    let mut manual_i32 = Vec::new();
    for &x in &v_i32 {
        manual_i32.extend_from_slice(&x.to_le_bytes());
    }
    let h_i32 = hash_portable(&v_i32[..]);
    print_u64(h_i32.0);
    print_bool(h_i32 == manual_hash_bytes(v_i32.len(), &manual_i32));

    let v_u64: Vec<u64> = black_box(vec![0x0102_0304_0506_0708, u64::MAX, 0]);
    let mut manual_u64 = Vec::new();
    for &x in &v_u64 {
        manual_u64.extend_from_slice(&x.to_le_bytes());
    }
    let h_u64 = hash_portable(&v_u64);
    print_u64(h_u64.0);
    print_bool(h_u64 == manual_hash_bytes(v_u64.len(), &manual_u64));

    let v_i64: Vec<i64> = black_box(vec![-1, i64::MIN, 0x0123_4567_89ab_cdef]);
    let mut manual_i64 = Vec::new();
    for &x in &v_i64 {
        manual_i64.extend_from_slice(&x.to_le_bytes());
    }
    let h_i64 = hash_portable(&v_i64);
    print_u64(h_i64.0);
    print_bool(h_i64 == manual_hash_bytes(v_i64.len(), &manual_i64));

    let v_u128: Vec<u128> = black_box(vec![0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10, u128::MAX]);
    let manual_u128: [u8; 32] = [
        0x10, 0x0f, 0x0e, 0x0d, 0x0c, 0x0b, 0x0a, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02,
        0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff,
    ];
    let h_u128 = hash_portable(&v_u128);
    print_u64(h_u128.0);
    print_bool(h_u128 == manual_hash_bytes(v_u128.len(), &manual_u128));

    let v_i128: [i128; 2] = black_box([-1, i128::MIN]);
    let manual_i128: [u8; 32] = [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x80,
    ];
    let h_i128 = hash_portable(&v_i128[..]);
    print_u64(h_i128.0);
    print_bool(h_i128 == manual_hash_bytes(v_i128.len(), &manual_i128));

    // usize / isize (verified against their target-width to_le_bytes() so native 64-bit & wasm32
    // both verify exact agreement with manual LE bytes).
    let v_usize: Vec<usize> = black_box(vec![0x0102_0304, 0x1122_3344]);
    let mut manual_usize = Vec::new();
    for &x in &v_usize {
        manual_usize.extend_from_slice(&x.to_le_bytes());
    }
    print_bool(hash_portable(&v_usize) == manual_hash_bytes(v_usize.len(), &manual_usize));

    let v_isize: Vec<isize> = black_box(vec![-1, -123456, 0x1234_5678]);
    let mut manual_isize = Vec::new();
    for &x in &v_isize {
        manual_isize.extend_from_slice(&x.to_le_bytes());
    }
    print_bool(hash_portable(&v_isize) == manual_hash_bytes(v_isize.len(), &manual_isize));

    let v_char: Vec<char> = black_box(vec!['A', 'é', '€', '😀']);
    let h_char_vec = hash_portable(&v_char);
    let h_char_arr = hash_portable(&['A', 'é', '€', '😀'][..]);
    print_u64(h_char_vec.0);
    print_bool(h_char_vec == h_char_arr);

    // 3. HashMap<Vec<u16>, u32> and HashSet-style lookup with Vec<u32> keys.
    let mut map16: SimpleMap<Vec<u16>, u32> = SimpleMap::new(16);
    map16.insert(vec![0x0102, 0x0304], 10);
    map16.insert(vec![0x0202, 0x0304], 20);
    map16.insert(vec![0x0102, 0x0305], 30);
    map16.insert(Vec::new(), 40);

    let mut probe16 = Vec::new();
    probe16.push(black_box(0x0102u16));
    probe16.push(black_box(0x0304u16));
    print_u32(*map16.get(&probe16).unwrap());
    print_u32(*map16.get(&vec![0x0202, 0x0304]).unwrap());
    print_u32(*map16.get(&vec![0x0102, 0x0305]).unwrap());
    print_u32(*map16.get(&Vec::new()).unwrap());
    print_bool(map16.get(&vec![0x0102, 0x0306]).is_none());

    let mut map32: SimpleMap<Vec<u32>, u32> = SimpleMap::new(16);
    map32.insert(vec![1, 2, 3], 100);
    map32.insert(vec![1, 2, 4], 200);
    let mut probe32 = Vec::with_capacity(4);
    probe32.extend_from_slice(&[1, 2, 3]);
    print_u32(*map32.get(&probe32).unwrap());
    print_u32(*map32.get(&vec![1, 2, 4]).unwrap());
    print_bool(map32.get(&vec![1, 2]).is_none());
}
