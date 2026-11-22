//! Host-delegated `slice::sort*` and `slice::sort_unstable*`: primitives, structs, indirect heap
//! values (`Box`/`String`/`Vec`), ZSTs, non-zero-offset subslices, stability, `sort_by_key`,
//! `sort_by_cached_key`, `fn` pointers, `dyn FnMut`, and inconsistent/NaN comparators.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;
use prelude::{print_bool, print_i32, print_str, print_usize};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pair {
    key: i32,
    tag: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HeapItem {
    key: i32,
    label: String,
    boxed: Box<i32>,
    nums: Vec<i32>,
}

fn cmp_i32_desc(a: &i32, b: &i32) -> Ordering {
    b.cmp(a)
}

#[inline(never)]
fn sort_with_dyn(slice: &mut [i32], cmp: &mut dyn FnMut(&i32, &i32) -> Ordering) {
    slice.sort_by(|a, b| cmp(a, b));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Primitives: sort & sort_unstable
    let mut nums = vec![9, 1, 5, 3, 7, 2, 8, 4, 6, 0, -5, 100, -5, 3];
    nums.sort();
    print_bool(nums == vec![-5, -5, 0, 1, 2, 3, 3, 4, 5, 6, 7, 8, 9, 100]);

    let mut bytes = vec![200u8, 10, 255, 0, 128, 10, 42, 1];
    bytes.sort_unstable();
    print_bool(bytes == vec![0, 1, 10, 10, 42, 128, 200, 255]);

    // 2. Non-zero offset subslice (off > 0)
    let mut sub = vec![99, 50, 40, 30, 20, 10, 1];
    sub[2..6].sort();
    print_bool(sub == vec![99, 50, 10, 20, 30, 40, 1]);

    // 3. Stability on equal keys (including larger n > 20 across merge levels)
    let mut pairs = Vec::new();
    for i in 0..40u32 {
        pairs.push(Pair {
            key: (39 - i) as i32 % 7,
            tag: i,
        });
    }
    pairs.sort_by_key(|p| p.key);
    let mut stable_ok = true;
    for w in pairs.windows(2) {
        if w[0].key > w[1].key || (w[0].key == w[1].key && w[0].tag >= w[1].tag) {
            stable_ok = false;
        }
    }
    print_bool(stable_ok);

    // 4. sort_by_cached_key
    let mut cached_calls = 0usize;
    let mut words = vec!["delta", "a", "charlie", "bb", "echo", "zz"];
    words.sort_by_cached_key(|s| {
        cached_calls += 1;
        s.len()
    });
    print_usize(cached_calls);
    print_bool(words == vec!["a", "bb", "zz", "echo", "delta", "charlie"]);

    // 5. Indirect heap structs (Box / String / Vec) + stability + sort_unstable_by_key
    let mut items = vec![
        HeapItem { key: 2, label: "first-2".to_string(), boxed: Box::new(20), nums: vec![1, 2] },
        HeapItem { key: 1, label: "only-1".to_string(), boxed: Box::new(10), nums: vec![3] },
        HeapItem { key: 2, label: "second-2".to_string(), boxed: Box::new(21), nums: vec![4, 5, 6] },
        HeapItem { key: 0, label: "zero".to_string(), boxed: Box::new(0), nums: vec![] },
        HeapItem { key: 2, label: "third-2".to_string(), boxed: Box::new(22), nums: vec![7] },
    ];
    items.sort_by(|a, b| a.key.cmp(&b.key));
    print_str(&items[0].label);
    print_str(&items[1].label);
    print_str(&items[2].label);
    print_str(&items[3].label);
    print_str(&items[4].label);
    print_i32(*items[2].boxed + *items[3].boxed + *items[4].boxed);

    items.sort_unstable_by_key(|it| -(*it.boxed));
    print_i32(*items[0].boxed);
    print_i32(*items[4].boxed);

    // 6. ZST slices
    let mut zsts = [(); 8];
    zsts.sort();
    zsts.sort_unstable();
    print_usize(zsts.len());

    // 7. Function pointer & dyn FnMut comparators
    let mut fp_nums = vec![3, 1, 4, 1, 5, 9, 2];
    let f: fn(&i32, &i32) -> Ordering = cmp_i32_desc;
    fp_nums.sort_by(f);
    print_bool(fp_nums == vec![9, 5, 4, 3, 2, 1, 1]);

    let mut dyn_cmp = |a: &i32, b: &i32| a.cmp(b);
    sort_with_dyn(&mut fp_nums, &mut dyn_cmp);
    print_bool(fp_nums == vec![1, 1, 2, 3, 4, 5, 9]);

    // 8. Inconsistent / NaN comparator: must terminate and preserve exact multiset
    let mut floats = vec![3.0f64, f64::NAN, 1.0, 2.0, f64::NAN, -1.0, 0.5];
    floats.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mut nan_count = 0usize;
    let mut finite_sum = 0.0f64;
    for &x in &floats {
        if x.is_nan() {
            nan_count += 1;
        } else {
            finite_sum += x;
        }
    }
    print_usize(nan_count);
    print_bool((finite_sum - 5.5).abs() < 1e-9);

    // Non-transitive rock-paper-scissors comparator: must remain a permutation of 0..9
    let mut rps: Vec<i32> = (0..9).collect();
    rps.sort_by(|a, b| match (a % 3, b % 3) {
        (0, 1) | (1, 2) | (2, 0) => Ordering::Less,
        (1, 0) | (2, 1) | (0, 2) => Ordering::Greater,
        _ => Ordering::Equal,
    });
    rps.sort();
    print_bool(rps == vec![0, 1, 2, 3, 4, 5, 6, 7, 8]);
}
