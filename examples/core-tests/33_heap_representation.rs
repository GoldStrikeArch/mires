//! Report-mode coverage for conservative heap-representation decisions.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

struct Pair {
    left: i32,
    right: i32,
}

#[inline(never)]
fn field_only(left: i32, right: i32) {
    let pair = Pair { left, right };
    print_i32(pair.left);
    print_i32(pair.right);
}

#[inline(never)]
fn aliased(value: i32) {
    let mut pair = Pair {
        left: value,
        right: 2,
    };
    let alias = &mut pair;
    alias.left += alias.right;
    print_i32(pair.left);
}

#[inline(never)]
fn raw_address(value: i32) {
    let pair = Pair {
        left: value,
        right: 4,
    };
    let pointer = &raw const pair;
    print_bool(!pointer.is_null());
    print_i32(pair.right);
}

struct Dropped(i32);

impl Drop for Dropped {
    fn drop(&mut self) {
        print_i32(self.0);
    }
}

#[inline(never)]
fn dropped(value: i32) {
    let value = Dropped(value);
    print_i32(value.0 + 1);
}

#[inline(never)]
fn capacities() {
    let mut values = Vec::with_capacity(4);
    values.push(7u32);
    print_usize(values.capacity());

    let mut text = String::with_capacity(8);
    text.push_str("rust");
    print_str(&text);

    let pair = Box::new(Pair { left: 8, right: 9 });
    print_i32(pair.left + pair.right);
}

#[no_mangle]
pub fn rust_entry() {
    field_only(1, 2);
    aliased(3);
    raw_address(5);
    dropped(6);
    capacities();
}
