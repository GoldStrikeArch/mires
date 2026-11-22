//! Safe `Box<str>` construction, conversions, spare-capacity `into_boxed_str`, and drop (MIRES-011).
//!
//! Covers `Box::<str>::from(&str)` (empty, ASCII, UTF-8), `From<&mut str>`, `String::into_boxed_str`
//! (exact-fit, spare capacity non-empty, spare capacity empty, `format!` builder conversion),
//! `Box::<str>::default`, `Box::<str>::clone`, `Box::into_string`, `Box::into_boxed_bytes`,
//! `Box::from_raw(Box::into_raw(b))`, and dropping every live heap allocation without aborting.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use core::hint::black_box;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Box::<str>::from(&str): empty, ASCII, multi-byte UTF-8 -> read & drop
    {
        let b_empty: Box<str> = Box::from(black_box(""));
        print_usize(b_empty.len());
        print_bool(b_empty.is_empty());
        print_str(&b_empty);
        drop(b_empty);

        let b_ascii: Box<str> = Box::from(black_box("hello"));
        print_usize(b_ascii.len());
        print_str(&b_ascii);
        drop(b_ascii);

        let b_utf8: Box<str> = Box::from(black_box("Héllo € 🚀"));
        print_usize(b_utf8.len());
        print_usize(b_utf8.chars().count());
        print_str(&b_utf8);
        drop(b_utf8);
    }

    // 2. String::into_boxed_str(): exact-fit, spare capacity (non-empty & empty), format!
    {
        let b1: Box<str> = String::from(black_box("exact-é")).into_boxed_str();
        print_str(&b1);
        drop(b1);

        let mut s_spare = String::with_capacity(black_box(64));
        s_spare.push_str(black_box("hi"));
        let b2: Box<str> = s_spare.into_boxed_str();
        print_usize(b2.len());
        print_str(&b2);
        drop(b2);

        let s_spare_empty = String::with_capacity(black_box(64));
        let b3: Box<str> = s_spare_empty.into_boxed_str();
        print_usize(b3.len());
        print_bool(b3.is_empty());
        drop(b3);

        let s_fmt = format!("x={}-é", black_box(42));
        let b4: Box<str> = s_fmt.into_boxed_str();
        print_str(&b4);
        drop(b4);
    }

    // 3. Box::<str>::default() -> read, mutate, into_string, drop
    {
        let mut b_def: Box<str> = Box::<str>::default();
        b_def.make_ascii_uppercase();
        print_usize(b_def.len());
        print_bool(b_def.is_empty());
        print_str(&b_def);
        drop(b_def);

        let b_def2: Box<str> = Box::<str>::default();
        let mut s_def = b_def2.into_string();
        s_def.push_str(black_box("grown"));
        print_str(s_def.as_str());
        drop(s_def);
    }

    // 4. Box::<str>::clone() -> mutate clone, verify original unchanged, drop both
    {
        let orig: Box<str> = Box::from(black_box("AbC-é"));
        let mut cloned = orig.clone();
        cloned.make_ascii_uppercase();
        print_str(&orig);
        print_str(&cloned);
        drop(cloned);
        drop(orig);
    }

    // 5. Box::into_string() and Box::into_boxed_bytes() -> mutate & drop
    {
        let mut s_spare = String::with_capacity(black_box(48));
        s_spare.push_str(black_box("AbC"));
        let mut b: Box<str> = s_spare.into_boxed_str();
        b.make_ascii_lowercase();
        let mut s: String = b.into_string();
        s.push_str(black_box("-é!"));
        print_str(s.as_str());
        drop(s);

        let b_bytes: Box<[u8]> = Box::<str>::from(black_box("xyz")).into_boxed_bytes();
        let mut b_bytes = b_bytes;
        b_bytes[0] = b'X';
        b_bytes[2] = b'Z';
        print_str(core::str::from_utf8(&b_bytes).unwrap());
        let v = b_bytes.into_vec();
        print_usize(v.len());
        drop(v);
    }

    // 6. Box::from_raw(Box::into_raw(b)) -> mutate through raw ptr & drop
    {
        let b: Box<str> = Box::from(black_box("rAw-é"));
        let raw: *mut str = Box::into_raw(b);
        let rebound: Box<str> = unsafe {
            (*raw).make_ascii_uppercase();
            Box::from_raw(raw)
        };
        print_str(&rebound);
        drop(rebound);
    }

    // 7. From<&mut str> for Box<str> -> independent buffers, drop both
    {
        let mut src = String::from(black_box("SoUrCe"));
        let mut boxed: Box<str> = Box::from(src.as_mut_str());
        src.make_ascii_lowercase();
        boxed.make_ascii_uppercase();
        print_str(src.as_str());
        print_str(&boxed);
        drop(boxed);
    }

    print_str("all_dropped_ok");
}
