//! Combined edge cases: sorting `Vec<String>` (native String elements), `sort_by_key` with
//! `to_lowercase()`, empty / 1-element / already-sorted / reverse-sorted / all-equal slices, and
//! `str::trim_matches` (pattern searcher over updated `Chars`).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use prelude::{print_bool, print_str, print_usize};

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Empty & 1-element slices
    let mut empty: Vec<i32> = Vec::new();
    empty.sort();
    empty.sort_unstable();
    let mut single = vec![42];
    single.sort();
    single.sort_unstable();
    print_bool(empty.is_empty() && single == vec![42]);

    // 2. Already-sorted, reverse-sorted, all-equal
    let mut sorted: Vec<i32> = (0..32).collect();
    sorted.sort();
    let mut rev: Vec<i32> = (0..32).rev().collect();
    rev.sort();
    let mut eq = vec![7i32; 25];
    eq.sort();
    print_bool(sorted == rev && eq.iter().all(|&x| x == 7));

    // 3. Vec<String> sorted case-insensitively with `to_lowercase`
    let mut names: Vec<String> = vec![
        "Zebra".to_string(),
        "apple".to_string(),
        "Banana".to_string(),
        "APPLE".to_string(),
        "ΟΣ".to_string(),
        "ος".to_string(),
    ];
    names.sort_by_key(|s| s.to_lowercase());
    print_str(&names[0]);
    print_str(&names[1]);
    print_str(&names[2]);
    print_str(&names[3]);
    print_str(&names[4]);
    print_str(&names[5]);

    // 4. Pattern searchers using `Chars` (`trim_matches`, `find`, `rfind`, `split`)
    let trimmed = "  \u{00a0}hello world\u{3000} ".trim_matches(|c: char| c.is_whitespace());
    print_str(trimmed);
    let parts: Vec<&str> = "a,b;c,d".split(|c| c == ',' || c == ';').collect();
    print_usize(parts.len());
    print_bool(parts == vec!["a", "b", "c", "d"]);
}
