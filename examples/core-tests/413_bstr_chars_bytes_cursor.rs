//! `str::chars()`, `str::bytes()`, `str::char_indices()`, `(&s[i..]).chars().next()` (`PERF-006`),
//! `Chars`/`Bytes` double-ended/exact-size/as_str/nth/advance_by methods, and long-string subslice
//! `_bsub_*` propagation (`PERF-003`).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::string::String;
use alloc::vec::Vec;
use prelude::{print_bool, print_str, print_u32, print_usize};

fn fnv1a(text: &str) -> u32 {
    text.bytes()
        .fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let sample = "aé€😀Z";

    // 1. Chars forward + as_str + size_hint + count
    let mut ch = sample.chars();
    print_usize(ch.clone().count());
    print_str(ch.as_str());
    let mut fwd = Vec::new();
    while let Some(c) = ch.next() {
        fwd.push(c);
    }
    print_bool(fwd == ['a', 'é', '€', '😀', 'Z']);
    print_bool(ch.as_str().is_empty());

    // 2. Chars reverse (next_back) + mixed front/back + advance_by
    let rev: Vec<char> = sample.chars().rev().collect();
    print_bool(rev == ['Z', '😀', '€', 'é', 'a']);

    let mut mix = sample.chars();
    print_bool(mix.next() == Some('a'));
    print_bool(mix.next_back() == Some('Z'));
    print_str(mix.as_str());
    print_bool(mix.nth(1) == Some('€'));
    print_str(mix.as_str());

    // 3. CharIndices forward & reverse + offset
    let ci_fwd: Vec<(usize, char)> = sample.char_indices().collect();
    print_bool(ci_fwd == [(0, 'a'), (1, 'é'), (3, '€'), (6, '😀'), (10, 'Z')]);
    let ci_rev: Vec<(usize, char)> = sample.char_indices().rev().collect();
    print_bool(ci_rev == [(10, 'Z'), (6, '😀'), (3, '€'), (1, 'é'), (0, 'a')]);

    // 4. Bytes forward, reverse, len, nth, nth_back, fold (pilot pattern)
    let mut bs = sample.bytes();
    print_usize(bs.len());
    print_bool(bs.next() == Some(b'a'));
    print_bool(bs.next_back() == Some(b'Z'));
    print_usize(bs.len());
    print_bool(bs.nth(2) == Some(0xe2));
    print_bool(bs.nth_back(1) == Some(0x98));
    print_u32(fnv1a("hello world"));

    // 5. PERF-006 pattern: `(&s[i..]).chars().next()` over a long multi-byte string
    let mut long = String::new();
    for _ in 0..40 {
        long.push_str("alpha-é-€-😀 ");
    }
    let s = long.as_str();
    let mut i = 0;
    let mut count = 0usize;
    let mut byte_sum = 0usize;
    while i < s.len() {
        let c = s[i..].chars().next().unwrap();
        let w = c.len_utf8();
        count += 1;
        byte_sum += s[i..].as_bytes()[0] as usize;
        i += w;
    }
    print_usize(count);
    print_usize(byte_sum);
    print_bool(count == s.chars().count());
}
