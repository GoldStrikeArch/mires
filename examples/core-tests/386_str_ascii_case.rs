//! ASCII case conversion on `str` and `String` (MIRES-025).
//!
//! Covers all four standard methods (`to_ascii_lowercase`, `to_ascii_uppercase`,
//! `make_ascii_lowercase`, `make_ascii_uppercase`) across ASCII, mixed-case, empty,
//! digits/punctuation, multibyte UTF-8 (Latin-1, Greek, Cyrillic, CJK, emoji),
//! `&mut String` via `DerefMut`, `&mut str`, `&mut s[a..b]`, `String` built via
//! `push_str`/`format!`/`with_capacity`, and Ky-style HTTP method normalization.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::format;
use alloc::string::String;
use core::hint::black_box;

const REQUEST_METHODS: [&str; 7] = ["get", "post", "put", "patch", "head", "delete", "query"];

#[inline(never)]
fn normalize_request_method(input: &str) -> String {
    if REQUEST_METHODS.contains(&input.to_ascii_lowercase().as_str()) {
        input.to_ascii_uppercase()
    } else {
        String::from(input)
    }
}

#[inline(never)]
fn lower_in_place(s: &mut str) {
    s.make_ascii_lowercase();
}

#[inline(never)]
fn upper_in_place(s: &mut str) {
    s.make_ascii_uppercase();
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. to_ascii_lowercase & to_ascii_uppercase on &str
    let samples: [&str; 6] = [
        "",
        "already lower 123!@#",
        "ALREADY UPPER 456",
        "The Quick BROWN Fox Jumps Over 13 Lazy Dogs!",
        "Café Ωmega Жизнь 東京 🦀 AbC-XyZ",
        "aAzZ09_-./",
    ];
    for sample in samples {
        let s = black_box(sample);
        let lo = s.to_ascii_lowercase();
        let hi = s.to_ascii_uppercase();
        print_str(lo.as_str());
        print_str(hi.as_str());
        // Original immutable &str must remain untouched.
        print_str(s);
    }

    // 2. make_ascii_lowercase & make_ascii_uppercase on String via DerefMut
    let mut s1 = String::from(black_box("HeLLo, WoRLd! 🦀 Éé Ωω"));
    s1.make_ascii_lowercase();
    print_str(s1.as_str());
    s1.make_ascii_uppercase();
    print_str(s1.as_str());

    // 3. Passing &mut str to #[inline(never)] functions
    let mut s2 = String::from(black_box("MiXeD-CaSe_Text_123"));
    lower_in_place(s2.as_mut_str());
    print_str(s2.as_str());
    upper_in_place(&mut s2);
    print_str(s2.as_str());

    // 4. Subslice mutation via &mut s[a..b]
    let mut s3 = String::from(black_box("AAA-bbb-CCC-ddd"));
    s3[0..3].make_ascii_lowercase();
    s3[4..7].make_ascii_uppercase();
    s3[8..11].make_ascii_lowercase();
    s3[12..15].make_ascii_uppercase();
    print_str(s3.as_str());

    // 5. String built with StringBuilder/HostString patterns (with_capacity, push_str, format!)
    let mut built = String::with_capacity(32);
    built.push_str(black_box("PreFix_"));
    built.push('A');
    built.push_str(black_box("bCd_🦀"));
    print_str(built.to_ascii_lowercase().as_str());
    print_str(built.to_ascii_uppercase().as_str());
    built.make_ascii_lowercase();
    print_str(built.as_str());

    let mut formatted = format!("{}:{}", black_box("Content-Type"), black_box("Text/Html"));
    let fmt_lo = formatted.to_ascii_lowercase();
    print_str(fmt_lo.as_str());
    formatted.make_ascii_uppercase();
    print_str(formatted.as_str());

    // 6. Ky-style HTTP method normalization
    for m in ["get", "PoSt", "DeLeTe", "HEAD", "CuStOm", "query"] {
        print_str(normalize_request_method(black_box(m)).as_str());
    }
}
