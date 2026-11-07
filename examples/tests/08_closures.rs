#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

fn apply<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 {
    f(x)
}

fn apply_twice<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 {
    f(f(x))
}

fn apply_mut<F: FnMut(i32)>(mut f: F) {
    f(1);
    f(2);
    f(3);
}

fn call_once_with<F: FnOnce(i32) -> i32>(f: F, x: i32) -> i32 {
    f(x)
}

fn zero_arg<F: Fn() -> i32>(f: F) -> i32 {
    f()
}

fn two_arg<F: Fn(i32, i32) -> i32>(f: F, a: i32, b: i32) -> i32 {
    f(a, b)
}

fn compose<F: Fn(i32) -> i32, G: Fn(i32) -> i32>(f: F, g: G, x: i32) -> i32 {
    g(f(x))
}

fn make_adder(n: i32) -> impl Fn(i32) -> i32 {
    move |x| x + n
}

#[no_mangle]
fn rust_entry() {
    // non-capturing closures
    print_i32(apply(|x| x * 2, 21));
    print_i32(apply_twice(|x| x + 3, 1));

    // capture by reference (immutable borrow); `base` stays usable afterwards
    let base: i32 = 10;
    let add_base = |x: i32| x + base;
    print_i32(apply(add_base, 5));
    print_i32(base);

    // capture by value with `move`
    let factor: i32 = 3;
    let scale = move |x: i32| x * factor;
    print_i32(apply(scale, 7));
    print_i32(factor);

    // arities other than one
    print_i32(two_arg(|a, b| a - b, 10, 4));
    let k: i32 = 99;
    print_i32(zero_arg(|| k + 1));

    // FnMut mutating a captured local
    let mut total: i32 = 0;
    {
        let accum = |x: i32| {
            total += x;
        };
        apply_mut(accum);
    }
    print_i32(total);

    let mut calls: i32 = 0;
    {
        let count = |_x: i32| {
            calls += 1;
        };
        apply_mut(count);
    }
    print_i32(calls);

    // FnMut reading and writing two captures
    let mut hi: i32 = 0;
    let mut lo: i32 = 100;
    {
        let track = |x: i32| {
            if x > hi {
                hi = x;
            }
            if x < lo {
                lo = x;
            }
        };
        apply_mut(track);
    }
    print_i32(hi);
    print_i32(lo);

    // FnOnce taking ownership of its capture
    let owned: i32 = 8;
    print_i32(call_once_with(move |x| x * owned, 5));

    // closure returned from a function
    let add5 = make_adder(5);
    print_i32(add5(1));
    print_i32(add5(10));
    print_i32(apply(add5, 100));

    // composition of two closures
    print_i32(compose(|x| x + 1, |x| x * 10, 4));

    // a closure calling another closure
    let inner = |x: i32| x * x;
    let outer = |x: i32| inner(x) + 1;
    print_i32(apply(outer, 6));

    print_str("08_closures ok");
}
