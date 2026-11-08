//! Similarly named methods on an unrelated impl are ordinary Rust operations.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::{string::String, vec::Vec};

struct Lookalike;

impl Lookalike {
    #[inline(never)]
    fn len(values: &Vec<u32>) -> usize {
        print_u32(101);
        values.len() + 7
    }

    #[inline(never)]
    fn clear(value: &mut String) {
        print_u32(102);
        value.push_str("!");
    }

    #[inline(never)]
    fn with_capacity(capacity: usize) -> String {
        print_u32(103);
        let mut result = String::with_capacity(capacity);
        result.push_str("kept");
        result
    }

    #[inline(never)]
    fn swap(values: &mut [u32], first: usize, second: usize) {
        print_u32(104);
        values[first] += values[second];
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut values = Vec::new();
    values.push(3);
    values.push(5);
    print_usize(Lookalike::len(&values));
    let mut text = Lookalike::with_capacity(8);
    Lookalike::clear(&mut text);
    print_usize(text.len());
    Lookalike::swap(values.as_mut_slice(), 0, 1);
    print_u32(values[0]);
    print_u32(values[1]);
}
