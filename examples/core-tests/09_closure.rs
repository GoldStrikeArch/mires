//! Closures against real `core`: the three `Fn` traits, captures by reference and by value,
//! closures as arguments and as return values, and the `core` combinators that take them.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

fn apply<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 {
    f(x)
}

fn apply_mut<F: FnMut() -> i32>(mut f: F) -> i32 {
    f() + f()
}

fn apply_once<F: FnOnce() -> i32>(f: F) -> i32 {
    f()
}

fn make_adder(n: i32) -> impl Fn(i32) -> i32 {
    move |x| x + n
}

/// A closure through a function pointer, which is a reified item rather than an environment.
fn apply_pointer(f: fn(i32) -> i32, x: i32) -> i32 {
    f(x)
}

fn double(x: i32) -> i32 {
    x * 2
}

struct Owned(i32);

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // `Fn`: a capture by shared reference.
    let base = 10;
    let add_base = |x: i32| x + base;
    print_i32(add_base(5));
    print_i32(apply(add_base, 7));
    print_i32(apply(|x| x * x, 6));

    // `FnMut`: a capture by mutable reference.
    let mut counter = 0;
    let mut bump = || {
        counter += 1;
        counter
    };
    print_i32(bump());
    print_i32(bump());
    print_i32(apply_mut(bump));
    print_i32(counter);

    // `FnOnce`: a capture by value that the call consumes.
    let owned = Owned(42);
    print_i32(apply_once(move || owned.0));

    // A closure returned from a function, with a `move` capture.
    let add_three = make_adder(3);
    print_i32(add_three(4));
    print_i32(apply(&add_three, 10));

    // Function pointers, both from an item and from a non-capturing closure.
    print_i32(apply_pointer(double, 8));
    print_i32(apply_pointer(|x| x - 1, 8));
    let table: [fn(i32) -> i32; 2] = [double, |x| x + 100];
    print_i32(table[0](3));
    print_i32(table[1](3));

    // `core`'s combinators, which is where closures meet the library.
    print_i32(Some(4).map(|x| x * 5).unwrap());
    print_i32(Some(4).filter(|x| *x > 3).map(add_base).unwrap());
    print_i32(None::<i32>.unwrap_or_else(|| base * 2));
    print_i32((1..6).map(|x| x * x).filter(|x| x % 2 == 1).sum());
    print_i32((1..6).fold(0, |acc, x| acc * 2 + x));
    print_bool((1..6).any(|x| x == base - 7));

    // A closure that captures another closure.
    let outer = |x: i32| apply(&add_three, x) * 2;
    print_i32(outer(1));

    // Recursion through a function pointer stored in a local.
    fn fib(n: i32) -> i32 {
        if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
    }
    let f: fn(i32) -> i32 = fib;
    print_i32(f(10));
}
