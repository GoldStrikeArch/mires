#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

// `<[T]>::len` lives in `core`, and `a..b` needs `Range` plus `SliceIndex`, so neither
// is available here. Length comes from `mini_core::slice_len`, which reads the fat
// pointer's metadata through the `ptr_metadata` intrinsic; sub-slicing comes from slice
// PATTERNS (`[a, rest @ ..]`), which lower to MIR `Subslice` projections without needing
// any library support. Iteration is a `while` loop over a manual `usize` counter.

fn sum(s: &[i32]) -> i32 {
    let mut total = 0;
    let mut i: usize = 0;
    while i < slice_len(s) {
        total += s[i];
        i += 1;
    }
    total
}

fn first_or(s: &[i32], fallback: i32) -> i32 {
    match s {
        [head, ..] => *head,
        [] => fallback,
    }
}

// `rest @ ..` is a `Subslice` projection; `last` is a from-the-end `ConstantIndex`.
fn ends(s: &[i32]) -> i32 {
    match s {
        [first, rest @ .., last] => *first * 100 + (slice_len(rest) as i32) * 10 + *last,
        [only] => *only,
        [] => -1,
    }
}

fn double_all(s: &mut [i32]) {
    let mut i: usize = 0;
    let n = slice_len(s);
    while i < n {
        s[i] *= 2;
        i += 1;
    }
}

fn max_of(s: &[i32]) -> i32 {
    let mut best = s[0];
    let mut i: usize = 1;
    while i < slice_len(s) {
        if s[i] > best {
            best = s[i];
        }
        i += 1;
    }
    best
}

#[no_mangle]
fn rust_entry() {
    let arr = [4i32, 8, 15, 16, 23];

    // Unsizing coercion `&[i32; 5]` -> `&[i32]`.
    let s: &[i32] = &arr;

    // Length from the fat pointer's metadata.
    print_i32(slice_len(s) as i32);

    // Indexing with constant and computed indices.
    print_i32(s[0]);
    print_i32(s[2]);
    print_i32(s[4]);
    let i: usize = 1;
    print_i32(s[i]);
    print_i32(s[i + 2]);

    // Passing `&[i32]` to a function, both from a slice and by coercing at the call.
    print_i32(sum(s));
    print_i32(sum(&arr));
    print_i32(max_of(s));

    // Slice patterns: head, subslice, from-the-end index.
    print_i32(first_or(s, -1));
    print_i32(ends(s));

    // A shorter array coerced through the same call sites.
    let two = [7i32, 9];
    let t: &[i32] = &two;
    print_i32(slice_len(t) as i32);
    print_i32(sum(t));
    print_i32(ends(t));

    let one = [3i32];
    print_i32(ends(&one));
    print_i32(first_or(&one, -1));

    // The empty slice: a coercion whose metadata is zero.
    let none: [i32; 0] = [];
    let e: &[i32] = &none;
    print_i32(slice_len(e) as i32);
    print_i32(sum(e));
    print_i32(first_or(e, -1));
    print_i32(ends(e));

    // `&mut [i32]`: writes through the slice are visible in the backing array.
    let mut marr = [1i32, 2, 3, 4];
    {
        let ms: &mut [i32] = &mut marr;
        ms[0] = 100;
        double_all(ms);
    }
    print_i32(marr[0]);
    print_i32(marr[1]);
    print_i32(marr[3]);
    print_i32(sum(&marr));

    // A slice held in a struct field.
    struct Window<'a> {
        data: &'a [i32],
        weight: i32,
    }
    let w = Window { data: s, weight: 2 };
    print_i32(sum(w.data) * w.weight);
    print_i32(w.data[1]);
    print_i32(slice_len(w.data) as i32);

    print_str("14_slices ok");
}
