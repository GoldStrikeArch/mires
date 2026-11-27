//! `rustc/tests/ui`-style tests for ZSTs, slices, UTF-8 `str`, `const` eval/promotion, numerics, and collections.

use std::collections::{HashMap, HashSet, VecDeque};
use std::marker::PhantomData;

use mires_oracle_rustc_ui::SplitMix64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ZeroSized(PhantomData<u64>, [u8; 0]);

#[test]
fn ui_zst_vec_box_and_slices() {
    let mut v = Vec::new();
    for _ in 0..1_000 {
        v.push(ZeroSized(PhantomData, []));
    }
    assert_eq!(v.len(), 1_000);
    assert_eq!(v.pop(), Some(ZeroSized(PhantomData, [])));
    assert_eq!(v.len(), 999);

    let b = Box::new(ZeroSized(PhantomData, []));
    assert_eq!(*b, ZeroSized(PhantomData, []));

    let empty: &[ZeroSized] = &v[10..10];
    assert!(empty.is_empty());
    assert_eq!(v[10..25].len(), 15);
}

const fn fib_const(n: u32) -> u64 {
    let mut a = 0u64;
    let mut b = 1u64;
    let mut i = 0;
    while i < n {
        let next = a + b;
        a = b;
        b = next;
        i += 1;
    }
    a
}

const FIB_TABLE: [u64; 10] = [
    fib_const(0),
    fib_const(1),
    fib_const(2),
    fib_const(3),
    fib_const(4),
    fib_const(5),
    fib_const(6),
    fib_const(7),
    fib_const(8),
    fib_const(9),
];

static PROMOTED_SLICE: &[i32] = &[3, 1, 4, 1, 5, 9];

#[test]
fn ui_const_eval_and_promotion() {
    assert_eq!(FIB_TABLE, [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]);
    assert_eq!(PROMOTED_SLICE.iter().sum::<i32>(), 23);

    let promoted: &'static [u32] = &[10, 20, 30];
    assert_eq!(promoted.len(), 3);
    assert_eq!(promoted[2], 30);
}

#[test]
fn ui_slices_sort_search_rotate_and_copy_within() {
    let mut rng = SplitMix64::new(0x5eed_cafe_1234_5678);
    let mut data: Vec<i32> = (0..200).map(|_| (rng.next_u64() as i32) % 1000).collect();
    let mut data2 = data.clone();

    data.sort();
    data2.sort_unstable();
    assert_eq!(data, data2);
    assert!(data.windows(2).all(|w| w[0] <= w[1]));

    let target = data[123];
    assert!(data.binary_search(&target).is_ok());

    let mut ring = [1, 2, 3, 4, 5, 6];
    ring.reverse();
    assert_eq!(ring, [6, 5, 4, 3, 2, 1]);
    ring.swap(0, 5);
    assert_eq!(ring, [1, 5, 4, 3, 2, 6]);

    let mut buf = *b"abcdefghij";
    buf.copy_within(2..6, 4);
    assert_eq!(&buf, b"abcdcdefij");
}

#[test]
fn ui_utf8_str_operations() {
    let s = "hello 🌍 世界 café!";
    assert_eq!(s.chars().count(), 16);
    assert!(s.is_char_boundary(0));
    assert!(s.is_char_boundary(6));
    assert!(!s.is_char_boundary(7)); // inside 🌍

    let parts: Vec<&str> = s.split_whitespace().collect();
    assert_eq!(parts, vec!["hello", "🌍", "世界", "café!"]);
    assert_eq!(s.replace("世界", "world"), "hello 🌍 world café!");
    assert_eq!("  \tαβγ\n ".trim(), "αβγ");
    assert_eq!("abc_def_ghi".split_once('_'), Some(("abc", "def_ghi")));
}

#[test]
fn ui_integer_widths_including_64_and_128_bit() {
    assert_eq!(u128::MAX.wrapping_add(1), 0);
    assert_eq!(i128::MIN.wrapping_neg(), i128::MIN);
    assert_eq!(i128::MIN.checked_neg(), None);
    assert_eq!(i64::MAX.saturating_add(10), i64::MAX);
    assert_eq!(i64::MIN.saturating_sub(10), i64::MIN);

    let big: u128 = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;
    assert_eq!(big.count_ones(), 64);
    assert_eq!(big.leading_zeros(), 7);
    assert_eq!(big.trailing_zeros(), 4);
    assert_eq!(big.swap_bytes().swap_bytes(), big);
    assert_eq!(big.rotate_left(37).rotate_right(37), big);

    let (q, r) = (
        1_000_000_000_000_000_000_007_u128 / 97,
        1_000_000_000_000_000_000_007_u128 % 97,
    );
    assert_eq!(q * 97 + r, 1_000_000_000_000_000_000_007_u128);
    assert_eq!((-17_i64).rem_euclid(5), 3);
    assert_eq!((-17_i64).div_euclid(5), -4);
}

#[test]
fn ui_floats_bits_nan_inf_and_total_cmp() {
    let pos_zero = 0.0_f64;
    let neg_zero = -0.0_f64;
    assert_eq!(pos_zero, neg_zero);
    assert_ne!(pos_zero.to_bits(), neg_zero.to_bits());
    assert_eq!(f64::from_bits(neg_zero.to_bits()).is_sign_negative(), true);
    assert!(f64::NAN.is_nan());
    assert_ne!(
        std::hint::black_box(f64::NAN),
        std::hint::black_box(f64::NAN)
    );
    assert!(f64::INFINITY.is_infinite());

    let mut vals = [f64::INFINITY, -0.0, 1.5, f64::NEG_INFINITY, 0.0, -2.5];
    vals.sort_by(f64::total_cmp);
    assert_eq!(vals[0], f64::NEG_INFINITY);
    assert_eq!(vals[1], -2.5);
    assert!(vals[2].is_sign_negative() && vals[2] == 0.0);
    assert!(vals[3].is_sign_positive() && vals[3] == 0.0);
    assert_eq!(vals[4], 1.5);
    assert_eq!(vals[5], f64::INFINITY);

    let f32_bits = (-3.25_f32).to_bits();
    assert_eq!(f32::from_bits(f32_bits), -3.25_f32);
}

#[test]
fn ui_std_collections_roundtrip() {
    let mut map: HashMap<String, i32> = HashMap::new();
    for i in 0..64 {
        map.insert(format!("k{i}"), i * i);
    }
    assert_eq!(map.len(), 64);
    assert_eq!(map.get("k12"), Some(&144));
    assert_eq!(map.remove("k12"), Some(144));
    assert_eq!(map.len(), 63);
    *map.entry(String::from("k10")).or_insert(0) += 5;
    assert_eq!(map.get("k10"), Some(&105));

    let set_a: HashSet<i32> = [1, 2, 3, 4, 5].into_iter().collect();
    let set_b: HashSet<i32> = [3, 4, 5, 6, 7].into_iter().collect();
    let mut inter: Vec<i32> = set_a.intersection(&set_b).copied().collect();
    inter.sort();
    assert_eq!(inter, vec![3, 4, 5]);

    let mut union_vals: Vec<i32> = set_a.union(&set_b).copied().collect();
    union_vals.sort();
    assert_eq!(union_vals, vec![1, 2, 3, 4, 5, 6, 7]);

    let mut dq = VecDeque::new();
    dq.push_back(2);
    dq.push_front(1);
    dq.push_back(3);
    dq.push_front(0);
    assert_eq!(dq.pop_front(), Some(0));
    assert_eq!(dq.pop_front(), Some(1));
    assert_eq!(dq.pop_back(), Some(3));
    assert_eq!(dq.pop_front(), Some(2));
    assert_eq!(dq.pop_front(), None);
}

#[test]
fn ui_iterators_and_adapters() {
    let out: Vec<i32> = (1..=20)
        .filter(|x| x % 2 == 0)
        .map(|x| x * x)
        .skip(2)
        .take(4)
        .collect();
    assert_eq!(out, vec![36, 64, 100, 144]);

    let zipped: Vec<(usize, char)> = ['r', 'u', 's', 't'].into_iter().enumerate().rev().collect();
    assert_eq!(zipped, vec![(3, 't'), (2, 's'), (1, 'u'), (0, 'r')]);

    let chunks: Vec<&[i32]> = [1, 2, 3, 4, 5].chunks(2).collect();
    assert_eq!(chunks, vec![&[1, 2][..], &[3, 4][..], &[5][..]]);

    let flat: Vec<i32> = [[1, 2], [3, 4], [5, 6]].into_iter().flatten().collect();
    assert_eq!(flat, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn ui_formatting_and_parsing() {
    assert_eq!(format!("{:#010x}", 0xdead_beef_u32), "0xdeadbeef");
    assert_eq!(format!("{:>6.2}", 3.14159_f64), "  3.14");
    assert_eq!(format!("{:08b}", 42_u8), "00101010");
    assert_eq!(format!("{:?}", vec![1, 2, 3]), "[1, 2, 3]");

    assert_eq!(
        "12345678901234567890".parse::<u128>(),
        Ok(12345678901234567890)
    );
    assert_eq!("-42.5".parse::<f64>(), Ok(-42.5));
    assert_eq!(i32::from_str_radix("ff", 16), Ok(255));
    assert!("not-a-num".parse::<i32>().is_err());
}

#[test]
fn ui_bit_and_endian_byte_conversions() {
    let x: u32 = 0x1234_5678;
    assert_eq!(x.to_le_bytes(), [0x78, 0x56, 0x34, 0x12]);
    assert_eq!(x.to_be_bytes(), [0x12, 0x34, 0x56, 0x78]);
    assert_eq!(u32::from_le_bytes([0x78, 0x56, 0x34, 0x12]), x);
    assert_eq!(u32::from_be_bytes([0x12, 0x34, 0x56, 0x78]), x);

    let y: i64 = -0x0102_0304_0506_0708;
    assert_eq!(i64::from_le_bytes(y.to_le_bytes()), y);
    assert_eq!(i64::from_be_bytes(y.to_be_bytes()), y);

    let z: u128 = 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10;
    assert_eq!(u128::from_be_bytes(z.to_be_bytes()), z);
    assert_eq!(u128::from_le_bytes(z.to_le_bytes()), z);
}
