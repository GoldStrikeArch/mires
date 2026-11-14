//! A caller that reaches one standard operation through native and baseline values must not
//! retarget both sites to the private native body.
//!
//! This is the two-statement shrink of representation-boundary seed 8. The exported round trip
//! makes the first `Vec` clone-eligible, while the function pointer keeps the second `Vec`
//! baseline. After inlining, both slice reads live in `rust_entry` and resolve to the same
//! `Vec::deref` item, so a caller-wide retarget would read the baseline vector as NativeVec.

#![no_std]
#![no_main]
#![allow(improper_ctypes)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

unsafe extern "C-unwind" {
    fn js_log_f64(x: f64);
    fn js_log_str(s: &str);
    fn js_abort(msg: &str) -> !;
}

fn print_usize(x: usize) {
    unsafe { js_log_f64(x as f64) }
}

fn print_i32(x: i32) {
    unsafe { js_log_f64(x as f64) }
}

fn print_str(s: &str) {
    unsafe { js_log_str(s) }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { js_abort("panic") }
}

static WORDS: [&str; 5] = ["alpha", "beta", "gamma", "delta", "h\u{e9}llo"];
static NUMS: [i32; 8] = [3, 1, 4, 1, 5, 9, 2, 6];

fn make_string(n: usize, separator: &str) -> String {
    let mut value = String::new();
    let mut index = 0;
    while index < n {
        value.push_str(WORDS[index % WORDS.len()]);
        value.push_str(separator);
        index += 1;
    }
    value
}

fn make_vec(n: usize) -> Vec<i32> {
    let mut values = Vec::with_capacity(n);
    let mut index = 0;
    while index < n {
        values.push(NUMS[index % NUMS.len()] + index as i32);
        index += 1;
    }
    values
}

fn consume_string(value: String) -> usize {
    value.len()
}

fn string_stats(value: &str) -> usize {
    let mut ascii = 0;
    for byte in value.as_bytes() {
        if *byte < 128 {
            ascii += 1;
        }
    }
    value.len() + ascii
}

fn sum_slice(values: &[i32]) -> i32 {
    let mut total = 0;
    for value in values {
        total += *value;
    }
    total
}

fn head_str(value: &str, n: usize) -> &str {
    let mut end = n.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn grow(value: &mut String, suffix: &str) {
    value.push_str(suffix);
    value.push('!');
}

fn push_all(values: &mut Vec<i32>, count: i32) {
    let mut value = 0;
    while value < count {
        values.push(value);
        value += 1;
    }
}

fn drain_half(values: &mut Vec<i32>) -> i32 {
    let mut total = 0;
    while values.len() > 1 {
        total += values.remove(0);
        if values.len() % 2 == 0 {
            break;
        }
    }
    total
}

#[unsafe(no_mangle)]
pub fn aux_vec_round_trip(mut values: Vec<i32>) -> Vec<i32> {
    values.push(7);
    values
}

#[unsafe(no_mangle)]
pub fn aux_string_round_trip(value: String) -> String {
    let mut value = value;
    value.push_str("~");
    value
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let values = aux_vec_round_trip(make_vec(2));
    print_usize(values.len());
    print_i32(sum_slice(&values));

    let call: fn(&mut Vec<i32>, i32) = push_all;
    let mut values = vec![0, 1];
    call(&mut values, 2);
    print_i32(sum_slice(&values));
}
