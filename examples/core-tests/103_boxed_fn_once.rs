//! Boxed closure dispatch through fat pointers, vtables, and captured environments.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::{String, ToString};

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

fn call0(callback: Box<dyn FnOnce() -> i32>) -> i32 {
    callback()
}

fn call1(callback: Box<dyn FnOnce(i32) -> i32>, value: i32) -> i32 {
    callback(value)
}

fn call2(callback: Box<dyn FnOnce(i32, i32) -> i32>, left: i32, right: i32) -> i32 {
    callback(left, right)
}

fn call_fn(callback: Box<dyn Fn(i32) -> i32>, value: i32) -> i32 {
    callback(value)
}

fn call_fn_mut(mut callback: Box<dyn FnMut(i32) -> i32>) -> i32 {
    callback(5) + callback(6)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let text: String = "abc".to_string();
    print_i32(call0(Box::new(move || text.len() as i32 + 4)));

    let base = 10;
    print_i32(call1(Box::new(move |value| base + value), 5));

    let weights = [2, 3];
    print_i32(call2(
        Box::new(move |left, right| weights[0] * left + weights[1] * right),
        4,
        5,
    ));

    let captured = 7;
    print_i32(call_fn(Box::new(move |value| captured + value), 5));

    let mut total = 3;
    print_i32(call_fn_mut(Box::new(move |value| {
        total += value;
        total
    })));
}
