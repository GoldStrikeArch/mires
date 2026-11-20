//! Closures called through `&dyn Fn`, `&mut dyn FnMut`, `Box<dyn FnOnce>` and generic bounds
//! with constant arguments, in callers with and without cleanup paths (MIRES-020).
//!
//! The call is the same `"rust-call"` call as a direct closure call; only the callee differs: a
//! vtable slot for the trait objects, the monomorphized closure body for the generic callers.
//! With `-Copt-level=z` the argument tuple is one constant, and a tuple whose layout is a single
//! scalar arrives as that scalar. Each caller is stamped out once holding a value with a
//! destructor across the calls and once holding a plain value.

#![no_std]
#![no_main]
#![feature(stmt_expr_attributes)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;

#[derive(Clone, Copy)]
struct Pair {
    hi: u32,
    lo: u32,
}

#[derive(Clone, Copy)]
struct Triple {
    a: u64,
    b: u32,
    c: bool,
}

struct Noisy(u32);

impl Drop for Noisy {
    fn drop(&mut self) {
        print_u32(9000 + self.0);
    }
}

impl Noisy {
    fn done(self) -> u32 {
        self.0
    }
}

struct Plain(u32);

impl Plain {
    fn done(self) -> u32 {
        print_u32(8000 + self.0);
        self.0
    }
}

fn show_pair(pair: Pair) {
    print_u32(pair.hi);
    print_u32(pair.lo);
}

fn show_triple(triple: Triple) {
    print_u64(triple.a);
    print_u32(triple.b);
    print_bool(triple.c);
}

/// Calls through trait objects: the callee is a vtable slot.
macro_rules! dyn_calls {
    ($name:ident, $guard:ident) => {
        #[inline(never)]
        fn $name(
            scalar: &dyn Fn(usize) -> u32,
            pair: &dyn Fn(usize) -> Pair,
            none: &dyn Fn() -> u32,
            three: &dyn Fn(u32, u64, bool) -> Triple,
            counter: &mut dyn FnMut(u32) -> Pair,
        ) {
            let guard = $guard(4);
            print_u32(scalar(2));
            show_pair(pair(1));
            print_u32(none());
            show_triple(three(7, 1 << 33, true));
            show_pair(counter(5));
            show_pair(counter(6));
            print_u32(guard.done());
        }
    };
}

/// Calls through generic bounds, one per closure trait.
macro_rules! generic_calls {
    ($fn_name:ident, $fn_mut_name:ident, $fn_once_name:ident, $guard:ident) => {
        #[inline(never)]
        fn $fn_name<F: Fn(usize) -> Pair>(f: &F) -> u32 {
            let guard = $guard(5);
            let pair = f(1);
            show_pair(pair);
            pair.hi + pair.lo + guard.done()
        }

        #[inline(never)]
        fn $fn_mut_name<F: FnMut(u32) -> u32>(mut f: F) -> u32 {
            let guard = $guard(6);
            let first = f(3);
            let second = f(4);
            print_u32(first);
            print_u32(second);
            second + guard.done()
        }

        #[inline(never)]
        fn $fn_once_name<F: FnOnce(u32, u32, u32) -> Triple>(f: F) -> u64 {
            let guard = $guard(7);
            let triple = f(1, 2, 3);
            show_triple(triple);
            triple.a + guard.done() as u64
        }
    };
}

dyn_calls!(dyn_cleanup, Noisy);
dyn_calls!(dyn_plain, Plain);
generic_calls!(via_fn_cleanup, via_fn_mut_cleanup, via_fn_once_cleanup, Noisy);
generic_calls!(via_fn_plain, via_fn_mut_plain, via_fn_once_plain, Plain);

#[inline(never)]
fn run_dyn(words: &[u32], cleanup: bool) {
    let scalar = |i: usize| words[i] * 10;
    let pair = |i: usize| Pair {
        hi: words[i],
        lo: words[i + 2],
    };
    let none = || words.len() as u32;
    let three = |a: u32, b: u64, c: bool| Triple {
        a: b + words[0] as u64,
        b: a,
        c: !c,
    };
    let mut seen = 0u32;
    let mut counter = |x: u32| {
        seen += x;
        Pair { hi: seen, lo: x }
    };
    let offset = words[3];
    let once: Box<dyn FnOnce(usize) -> Pair> = Box::new(move |i: usize| Pair {
        hi: offset,
        lo: i as u32,
    });
    if cleanup {
        dyn_cleanup(&scalar, &pair, &none, &three, &mut counter);
    } else {
        dyn_plain(&scalar, &pair, &none, &three, &mut counter);
    }
    print_u32(seen);
    // The box is this caller's cleanup until the call consumes it.
    show_pair(once(0));
}

#[inline(never)]
fn run_generic(words: &[u32]) {
    let pair = #[inline(never)]
    |i: usize| Pair {
        hi: words[i] + 100,
        lo: words[i + 1] + 200,
    };
    print_u32(via_fn_cleanup(&pair));
    print_u32(via_fn_plain(&pair));
    let mut sum = 0u32;
    print_u32(via_fn_mut_cleanup(
        #[inline(never)]
        |x: u32| {
            sum += x * words[1];
            sum
        },
    ));
    print_u32(via_fn_mut_plain(
        #[inline(never)]
        |x: u32| {
            sum += x;
            sum
        },
    ));
    print_u32(sum);
    let owned = Noisy(words[2]);
    print_u64(via_fn_once_cleanup(
        #[inline(never)]
        move |a: u32, b: u32, c: u32| Triple {
            a: (owned.done() + a) as u64,
            b: b * 10,
            c: c == 3,
        },
    ));
    let kept = Plain(words[3]);
    print_u64(via_fn_once_plain(
        #[inline(never)]
        move |a: u32, b: u32, c: u32| Triple {
            a: (kept.done() + a) as u64,
            b: b * 20,
            c: c == 4,
        },
    ));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let words = [3u32, 1, 4, 1, 5, 9];
    print_str("dyn");
    run_dyn(&words, true);
    run_dyn(&words, false);
    print_str("generic");
    run_generic(&words);
}
