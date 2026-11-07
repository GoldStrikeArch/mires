//! Sorting and the rest of the in place slice algorithms.
//!
//! `sort_unstable` is the deepest pointer code in `core` that a program is likely to reach:
//! ipnsort switches on the length, runs an insertion sort under 20 elements and a
//! pattern defeating quicksort above it, and every one of those moves values with
//! `ptr::copy_nonoverlapping` through a `MaybeUninit` scratch slot rather than with an
//! assignment. The sizes below straddle that switch on purpose — 16 takes the small path, 40
//! takes the partitioning one.
//!
//! `swap`, `reverse`, `copy_from_slice` and `starts_with` are here for the same reason:
//! `ptr::swap`, `ptr::copy_nonoverlapping` and `compare_bytes` are what `core` writes them with.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Item {
    key: i32,
    tag: i32,
}

/// A small linear congruential generator, so the unsorted input is the same in both worlds.
fn fill(values: &mut [i32]) {
    let mut state = 12345u32;
    let mut i = 0;
    while i < values.len() {
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        values[i] = ((state >> 16) % 100) as i32;
        i += 1;
    }
}

fn is_sorted(values: &[i32]) -> bool {
    let mut i = 1;
    while i < values.len() {
        if values[i - 1] > values[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// A single number that changes if any element moves, so a 40 element result is one line.
fn checksum(values: &[i32]) -> i32 {
    let mut acc = 7i32;
    let mut i = 0;
    while i < values.len() {
        acc = acc.wrapping_mul(31).wrapping_add(values[i]);
        i += 1;
    }
    acc
}

fn print_all(values: &[i32]) {
    let mut i = 0;
    while i < values.len() {
        print_i32(values[i]);
        i += 1;
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // ---------------------------------------------------------------- sort_unstable on [i32]
    // The degenerate lengths first: an empty and a one element slice must not touch anything.
    let mut none: [i32; 0] = [];
    none.sort_unstable();
    print_usize(none.len());
    print_bool(is_sorted(&none));

    let mut one = [42i32];
    one.sort_unstable();
    print_all(&one);

    let mut two = [2i32, 1];
    two.sort_unstable();
    print_all(&two);

    let mut two_sorted = [1i32, 2];
    two_sorted.sort_unstable();
    print_all(&two_sorted);

    // Sixteen elements: the insertion sort path.
    let mut small = [0i32; 16];
    fill(&mut small);
    print_i32(checksum(&small));
    small.sort_unstable();
    print_bool(is_sorted(&small));
    print_i32(checksum(&small));
    print_i32(small[0]);
    print_i32(small[15]);

    // Forty elements: the partitioning path.
    let mut large = [0i32; 40];
    fill(&mut large);
    print_i32(checksum(&large));
    large.sort_unstable();
    print_bool(is_sorted(&large));
    print_i32(checksum(&large));
    print_i32(large[0]);
    print_i32(large[39]);
    print_i32(large.iter().sum::<i32>());

    // Already sorted, and reverse sorted: the two patterns the algorithm detects.
    let mut ascending = [1i32, 2, 3, 4, 5, 6, 7, 8];
    ascending.sort_unstable();
    print_i32(checksum(&ascending));
    let mut descending = [8i32, 7, 6, 5, 4, 3, 2, 1];
    descending.sort_unstable();
    print_all(&descending);

    // ------------------------------------------------------------------------ sort_unstable_by
    let mut by = [5i32, 1, 4, 2, 3];
    by.sort_unstable_by(|a, b| b.cmp(a));
    print_all(&by);

    let mut by_u64 = [30u64, 10, 20];
    by_u64.sort_unstable();
    print_u64(by_u64[0]);
    print_u64(by_u64[2]);

    let mut wide = [0u64; 16];
    let mut i = 0;
    while i < wide.len() {
        wide[i] = ((wide.len() - i) as u64) * 1_000_000_007;
        i += 1;
    }
    wide.sort_unstable();
    print_u64(wide[0]);
    print_u64(wide[15]);
    let mut wide_sorted = true;
    let mut j = 1;
    while j < wide.len() {
        if wide[j - 1] > wide[j] {
            wide_sorted = false;
        }
        j += 1;
    }
    print_bool(wide_sorted);

    // --------------------------------------------------------------------- sort_unstable_by_key
    let mut items = [
        Item { key: 3, tag: 30 },
        Item { key: 1, tag: 10 },
        Item { key: 2, tag: 20 },
    ];
    items.sort_unstable_by_key(|item| item.key);
    print_i32(items[0].tag);
    print_i32(items[1].tag);
    print_i32(items[2].tag);

    // A struct element sorted by its derived `Ord`, at a length that partitions. This is the case
    // that broke last: an aggregate moved through the partition's scratch slot has to be *cloned*,
    // because assigning the JavaScript object would alias it, and a sort that aliases loses one
    // element and duplicates another. The keys are `(k * 37) % 41`, a permutation, so every key is
    // distinct and the sorted order is fully determined; the tag check below is what catches a
    // lost element.
    let mut records = [Item { key: 0, tag: 0 }; 40];
    let mut k = 0;
    while k < records.len() {
        records[k] = Item {
            key: ((k * 37) % 41) as i32,
            tag: k as i32,
        };
        k += 1;
    }
    records.sort_unstable();
    let mut records_sorted = true;
    let mut m = 1;
    while m < records.len() {
        if records[m - 1].key > records[m].key {
            records_sorted = false;
        }
        m += 1;
    }
    print_bool(records_sorted);
    print_i32(records[0].key);
    print_i32(records[0].tag);
    print_i32(records[39].key);
    print_i32(records[39].tag);

    // Every tag still present exactly once.
    let mut seen = [0i32; 40];
    let mut scan = 0;
    while scan < records.len() {
        seen[records[scan].tag as usize] += 1;
        scan += 1;
    }
    let mut all_once = true;
    let mut check = 0;
    while check < seen.len() {
        if seen[check] != 1 {
            all_once = false;
        }
        check += 1;
    }
    print_bool(all_once);

    // ------------------------------------------------------------------ swap, reverse, contains
    let mut values = [10i32, 20, 30, 40, 50];
    values.swap(0, 4);
    print_all(&values);
    values.swap(2, 2);
    print_i32(values[2]);

    values.reverse();
    print_all(&values);
    let mut odd = [1i32, 2, 3];
    odd.reverse();
    print_all(&odd);
    let mut nothing: [i32; 0] = [];
    nothing.reverse();
    print_usize(nothing.len());

    let mut structs = [Item { key: 1, tag: 1 }, Item { key: 2, tag: 2 }];
    structs.swap(0, 1);
    print_i32(structs[0].key);
    print_i32(structs[1].key);

    let haystack = [1i32, 3, 5, 7, 9];
    print_bool(haystack.contains(&7));
    print_bool(haystack.contains(&8));
    print_bool(<[i32]>::contains(&[], &1));

    // -------------------------------------------------------------------------- binary_search
    match haystack.binary_search(&5) {
        Ok(index) => {
            print_str("found");
            print_usize(index);
        }
        Err(index) => {
            print_str("insert");
            print_usize(index);
        }
    }
    match haystack.binary_search(&6) {
        Ok(index) => {
            print_str("found");
            print_usize(index);
        }
        Err(index) => {
            print_str("insert");
            print_usize(index);
        }
    }
    match haystack.binary_search(&0) {
        Ok(index) => {
            print_str("found");
            print_usize(index);
        }
        Err(index) => {
            print_str("insert");
            print_usize(index);
        }
    }
    print_bool(haystack.binary_search(&9).is_ok());
    print_bool(<[i32]>::binary_search(&[], &1).is_err());

    // ------------------------------------------------------- copy_from_slice, starts_with, fill
    let source = [1i32, 2, 3, 4];
    let mut destination = [0i32; 4];
    destination.copy_from_slice(&source);
    print_all(&destination);

    let mut partial = [9i32; 6];
    partial[2..6].copy_from_slice(&source);
    print_all(&partial);

    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&[7u8, 8, 9, 10]);
    print_i32(bytes[0] as i32);
    print_i32(bytes[3] as i32);

    print_bool(source.starts_with(&[1, 2]));
    print_bool(source.starts_with(&[2, 3]));
    print_bool(source.starts_with(&[]));
    print_bool(source.ends_with(&[3, 4]));
    print_bool(source.starts_with(&source));

    let mut filled = [0i32; 5];
    filled.fill(3);
    print_i32(checksum(&filled));
    let mut zeroed = [1u8; 4];
    zeroed.fill(0);
    print_i32(zeroed[0] as i32 + zeroed[3] as i32);
}
