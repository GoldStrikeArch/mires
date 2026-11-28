//! Scalar `memrchr` reverse byte search: `str::rsplit_once`, `rfind`, `rsplit`, `rsplitn`
//! across short, long (> 64 bytes), and Unicode strings.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec::Vec;
use core::hint::black_box;

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    let short = black_box("alpha/beta/gamma");
    let (l, r) = short.rsplit_once('/').unwrap();
    print_str(l);
    print_str(r);
    print_usize(short.rfind('/').unwrap());
    print_bool(short.rfind('z').is_none());

    // Long ASCII string (> 64 bytes, previously triggered align_to::<(usize, usize)> in memrchr)
    let long = black_box(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:tail_end",
    );
    let (ll, lr) = long.rsplit_once(':').unwrap();
    print_usize(ll.len());
    print_str(lr);
    print_usize(long.rfind(':').unwrap());

    let parts: Vec<&str> = long.rsplit(':').collect();
    print_usize(parts.len());
    print_str(parts[0]);
    print_usize(parts[1].len());
    print_usize(parts[2].len());

    let limited: Vec<&str> = long.rsplitn(2, ':').collect();
    print_usize(limited.len());
    print_str(limited[0]);
    print_usize(limited[1].len());

    // Unicode + long UTF-8 string
    let uni = black_box("αβγδεζηθικλμνξοπρστυφχψω/🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀/end-λ");
    let (ul, ur) = uni.rsplit_once('/').unwrap();
    print_str(ur);
    print_usize(ul.len());
    print_usize(uni.rfind('λ').unwrap());

    // Edge cases: empty, needle at byte 0, needle at last byte
    let empty = black_box("");
    print_bool(empty.rfind('a').is_none());
    print_bool(empty.rsplit_once('a').is_none());

    let edge = black_box("/middle/");
    print_usize(edge.rfind('/').unwrap());
    let (el, er) = edge.rsplit_once('/').unwrap();
    print_str(el);
    print_usize(er.len());
}
