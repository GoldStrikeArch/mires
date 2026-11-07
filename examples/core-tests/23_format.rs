//! `format!`, which is `core::fmt` writing into a `String` rather than into a borrowed sink.
//!
//! `examples/core-tests/11_fmt.rs` already pins the formatting machinery against a `fmt::Write`
//! implementation of its own. What is new here is the destination: `format!` allocates, grows and
//! decodes a heap block, so every width, fill and precision below is also a test of the block
//! surviving the writes.
//!
//! The last section is the other destination -- the host string builder `prelude::JsStr` wraps
//! (`CONTRACT.md`, "Allocation") -- which is what a program should reach for when it is only going
//! to hand the text back to JavaScript anyway.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::format;
use alloc::string::String;
use core::fmt;
use core::fmt::Write;

struct Point {
    x: i32,
    y: i32,
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

struct Tag;

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("tag")
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // The plain interpolations.
    print_str(&format!("{}", 42));
    print_str(&format!("{} and {}", 1, 2));
    print_str(&format!("{0}-{1}-{0}", "a", "b"));
    print_str(&format!("{}", true));
    print_str(&format!("{}", 'z'));

    // Width, alignment and fill.
    print_str(&format!("[{:5}]", 42));
    print_str(&format!("[{:<5}]", 42));
    print_str(&format!("[{:>5}]", 42));
    print_str(&format!("[{:^5}]", 42));
    print_str(&format!("[{:*^7}]", "ab"));
    print_str(&format!("[{:05}]", 42));
    print_str(&format!("[{:+}]", 42));

    // Radix.
    print_str(&format!("{:x} {:X} {:o} {:b}", 255, 255, 8, 5));
    print_str(&format!("{:#x} {:#o} {:#b}", 255, 8, 5));
    print_str(&format!("[{:#010x}]", 255));

    // Floats: precision, and the default.
    print_str(&format!("{}", 1.5));
    print_str(&format!("{:.3}", 1.0 / 3.0));
    print_str(&format!("{:.0}", 2.5));
    print_str(&format!("[{:8.2}]", 3.14159));
    print_str(&format!("{:e}", 1234.5));

    // A user `Display`, including one that writes through `write!` itself.
    print_str(&format!("{}", Point { x: 3, y: -4 }));
    print_str(&format!("[{:>10}]", Point { x: 1, y: 2 }));
    print_str(&format!("{}-{}", Tag, Tag));

    // Growth: a format long enough that the block reallocates more than once.
    let mut long = String::new();
    let mut i = 0;
    while i < 20 {
        long.push_str(&format!("{},", i));
        i += 1;
    }
    print_usize(long.len());
    print_str(&long);

    // `write!` into a `String`, which is the same sink `format!` uses.
    let mut into_string = String::new();
    write!(into_string, "{}+{}", 6, 7).unwrap();
    print_str(&into_string);

    // `write!` into the host string builder: no heap block at all, and the result is already the
    // JavaScript string every shim member takes.
    let mut sink = JsStr::new();
    write!(sink, "{} {:>4} {:.2}", Tag, 42, 1.5).unwrap();
    sink.write_char('!').unwrap();
    print_str(sink.take());

    // The sink is empty again, and reusable.
    write!(sink, "{}", Point { x: 8, y: 9 }).unwrap();
    print_str(sink.take());
}
