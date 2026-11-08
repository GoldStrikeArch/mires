//! Float operands execute once, from left to right, even when their consumer branches.

#![no_std]
#![no_main]
#![feature(coerce_unsized, unsize)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn observe(value: f64) -> f64 {
    print_f64(value);
    value
}

#[no_mangle]
pub fn observed_min(left: f64, right: f64) -> f64 {
    observe(left).min(observe(right))
}

#[no_mangle]
pub fn observed_max(left: f64, right: f64) -> f64 {
    observe(left).max(observe(right))
}

#[no_mangle]
pub fn observed_copysign(left: f64, right: f64) -> f64 {
    observe(left).copysign(observe(right))
}

#[no_mangle]
pub fn observed_min32(left: f64, right: f64) -> f32 {
    (observe(left) as f32).min(observe(right) as f32)
}

#[no_mangle]
pub fn observed_max32(left: f64, right: f64) -> f32 {
    (observe(left) as f32).max(observe(right) as f32)
}

#[no_mangle]
pub fn observed_copysign32(left: f64, right: f64) -> f32 {
    (observe(left) as f32).copysign(observe(right) as f32)
}

#[no_mangle]
pub fn observed_cmp(left: f64, right: f64) -> i32 {
    (observe(left) as i32).cmp(&(observe(right) as i32)) as i32
}

#[no_mangle]
pub fn observed_and(left: f64, right: f64) -> bool {
    (observe(left) != 0.0) & (observe(right) != 0.0)
}

#[no_mangle]
pub fn observed_or(left: f64, right: f64) -> bool {
    (observe(left) != 0.0) | (observe(right) != 0.0)
}

#[inline(never)]
fn observe_pair(left: f64, right: f64) -> (f64, f64) {
    (observe(left), observe(right))
}

#[no_mangle]
pub fn observed_repeat(left: f64, right: f64) -> f64 {
    let rows = [observe_pair(left, right); 2];
    rows[0].0 + rows[1].1
}

#[no_mangle]
pub fn observed_repeat_empty(left: f64, right: f64) -> f64 {
    let rows = [observe_pair(left, right); 0];
    rows.len() as f64
}

enum Comparison {
    Less,
    Equal,
    Greater,
}

#[inline(never)]
fn observe_comparison(left: f64, right: f64) -> Comparison {
    let left = observe(left);
    let right = observe(right);
    if left < right {
        Comparison::Less
    } else if left == right {
        Comparison::Equal
    } else {
        Comparison::Greater
    }
}

#[no_mangle]
pub fn observed_discriminant(left: f64, right: f64) -> i32 {
    observe_comparison(left, right) as i32
}

struct WithPointer<T: ?Sized> {
    tag: i32,
    pointer: *const T,
}

impl<T: core::marker::Unsize<U> + ?Sized, U: ?Sized> core::ops::CoerceUnsized<WithPointer<U>>
    for WithPointer<T>
{
}

#[inline(never)]
fn observe_pointer(left: f64, right: f64) -> WithPointer<[i32; 2]> {
    static VALUES: [i32; 2] = [1, 2];
    let tag = observe(left) as i32;
    observe(right);
    WithPointer {
        tag,
        pointer: &VALUES,
    }
}

#[no_mangle]
pub fn observed_unsize(left: f64, right: f64) -> f64 {
    let value: WithPointer<[i32]> = observe_pointer(left, right);
    value.tag as f64 + value.pointer.len() as f64
}

#[no_mangle]
pub fn rust_entry() {
    print_f64(observed_min(7.0, 3.0));
    print_f64(observed_max(7.0, 3.0));
    print_f64(observed_copysign(7.0, -3.0));
    print_f64(observed_min32(7.0, 3.0) as f64);
    print_f64(observed_max32(7.0, 3.0) as f64);
    print_f64(observed_copysign32(7.0, -3.0) as f64);
    print_i32(observed_cmp(7.0, 3.0));
    print_bool(observed_and(0.0, 3.0));
    print_bool(observed_or(7.0, 3.0));
    print_f64(observed_repeat(7.0, 3.0));
    print_f64(observed_repeat_empty(7.0, 3.0));
    print_i32(observed_discriminant(7.0, 3.0));
    print_f64(observed_unsize(7.0, 3.0));
}
