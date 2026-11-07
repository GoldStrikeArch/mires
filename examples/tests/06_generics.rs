#![feature(no_core, lang_items, intrinsics, rustc_attrs, decl_macro)]
#![feature(auto_traits, freeze_impls, unboxed_closures)]
#![allow(internal_features, dead_code, unused_variables)]
#![no_core]
#![no_main]

#[path = "../mini_core.rs"]
mod mini_core;
use mini_core::*;

struct Pair<T> {
    a: T,
    b: T,
}

struct Wrapper<T> {
    value: T,
    count: i32,
}

impl<T: Copy> Pair<T> {
    fn first(&self) -> T {
        self.a
    }

    fn second(&self) -> T {
        self.b
    }

    fn swapped(&self) -> Pair<T> {
        Pair { a: self.b, b: self.a }
    }
}

fn identity<T>(x: T) -> T {
    x
}

fn pick<T>(cond: bool, a: T, b: T) -> T {
    if cond { a } else { b }
}

fn max_of<T: PartialOrd + Copy>(a: T, b: T) -> T {
    if a > b { a } else { b }
}

fn min_of<T: PartialOrd + Copy>(a: T, b: T) -> T {
    if a < b { a } else { b }
}

fn sum_pair<T: Add<Output = T> + Copy>(p: &Pair<T>) -> T {
    p.a + p.b
}

fn wrap<T>(value: T) -> Wrapper<T> {
    Wrapper { value, count: 1 }
}

#[no_mangle]
fn rust_entry() {
    // one generic fn, three instantiations
    print_i32(identity(42));
    print_f64(identity(2.5));
    print_bool(identity(true));

    print_i32(pick(true, 1, 2));
    print_i32(pick(false, 1, 2));
    print_f64(pick(true, 1.5, 9.5));

    // generic fn with a trait bound
    print_i32(max_of(3, 9));
    print_i32(max_of(-4, -9));
    print_f64(max_of(2.5, 1.25));
    print_bool(max_of(false, true));
    print_i32(min_of(3, 9));
    print_f64(min_of(2.5, 1.25));

    // generic struct, i32 instantiation
    let pi = Pair { a: 10, b: 32 };
    print_i32(pi.first());
    print_i32(pi.second());
    print_i32(sum_pair(&pi));
    let sw = pi.swapped();
    print_i32(sw.first());
    print_i32(sw.second());

    // generic struct, f64 instantiation
    let pf = Pair { a: 1.5, b: 2.25 };
    print_f64(pf.first());
    print_f64(pf.second());
    print_f64(sum_pair(&pf));
    print_f64(pf.swapped().first());

    // generic struct with a non-generic field
    let w = wrap(7);
    print_i32(w.value);
    print_i32(w.count);
    let wf = wrap(0.5);
    print_f64(wf.value);
    print_i32(wf.count);

    // nested generic instantiation
    let nested = wrap(Pair { a: 1, b: 2 });
    print_i32(sum_pair(&nested.value));

    print_str("06_generics ok");
}
