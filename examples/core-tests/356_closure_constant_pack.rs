//! A closure called with constant arguments, in a caller that has cleanup paths (MIRES-020).
//!
//! A call to a closure is a `"rust-call"` call whose last argument is the tuple of the closure's
//! arguments. When every argument is a constant, MIR's optimizations (`-Copt-level=z`) fold that
//! tuple into one constant. A tuple whose layout is a single scalar, such as `(usize,)` or
//! `(u32, ())`, then evaluates to that scalar rather than to memory, and the JIR encoder refused
//! to split it into the arguments (`SpreadNotTuple`). In a caller with cleanup paths, which
//! cannot fall back to the baseline lane, that refused the whole program.
//!
//! Every closure is kept out of line (`#[inline(never)]`), so its call survives the MIR inliner.
//! Each caller is stamped out twice: once holding a value with a destructor across the calls,
//! which gives it cleanup paths, and once holding a plain value, which gives it none. The cases
//! are `Fn`, `FnMut` and `FnOnce` closures with zero, one and three arguments, returning a scalar,
//! a pair of words and a larger aggregate, and the original shape: a pair read by index, then
//! consumed by an owning array iterator.

#![no_std]
#![no_main]
#![feature(stmt_expr_attributes)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Pair {
    hi: u32,
    lo: u32,
}

#[derive(Clone, Copy)]
struct Wide {
    a: u32,
    b: u64,
    c: u8,
    d: i16,
}

/// A value with a destructor: live across a call, it gives the caller a cleanup path.
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

/// The same value without a destructor.
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

fn show_wide(wide: Wide) {
    print_u32(wide.a);
    print_u64(wide.b);
    print_u32(wide.c as u32);
    print_i32(wide.d as i32);
}

/// The MIRES-020 shape: a capturing closure returns a pair of words read by index, and the pairs
/// are consumed by an owning iterator, whose drop is the caller's cleanup.
#[inline(never)]
fn sum_lanes(words: &[u32], index: usize) -> u32 {
    let unpack = #[inline(never)]
    |i: usize| Pair {
        hi: words[i * 2],
        lo: words[i * 2 + 1],
    };
    let a = unpack(index);
    let b = unpack(1);
    let mut sum = 0;
    for value in [a, b].into_iter() {
        sum += value.hi + value.lo;
    }
    sum
}

/// `Fn` closures with zero, one and three constant arguments.
macro_rules! fn_calls {
    ($name:ident, $guard:ident) => {
        #[inline(never)]
        fn $name(words: &[u32]) {
            let guard = $guard(1);
            let none = #[inline(never)]
            || words[0] + 7;
            let one = #[inline(never)]
            |i: usize| Pair {
                hi: words[i],
                lo: words[i + 1],
            };
            let three = #[inline(never)]
            |i: usize, scale: u64, d: i16| Wide {
                a: words[i],
                b: words[i + 1] as u64 * scale,
                c: words[i + 2] as u8,
                d,
            };
            print_u32(none());
            show_pair(one(1));
            show_pair(one(2));
            show_wide(three(0, 1 << 40, -3));
            print_u32(guard.done());
        }
    };
}

/// `FnMut` closures: the state they write is read back after the calls.
macro_rules! fn_mut_calls {
    ($name:ident, $guard:ident) => {
        #[inline(never)]
        fn $name(words: &[u32]) {
            let guard = $guard(2);
            let mut total = 0u32;
            let mut count = 0u32;
            let mut add = #[inline(never)]
            |i: usize| {
                total += words[i];
                count += 1;
                Pair {
                    hi: total,
                    lo: count,
                }
            };
            show_pair(add(0));
            show_pair(add(3));
            let mut bump = #[inline(never)]
            || {
                count += 10;
                count
            };
            print_u32(bump());
            print_u32(bump());
            let mut many = #[inline(never)]
            |a: u32, b: u32, c: u32| {
                total = total * 2 + a + b + c;
                total
            };
            print_u32(many(1, 2, 3));
            print_u32(total);
            print_u32(count);
            print_u32(guard.done());
        }
    };
}

/// `FnOnce` closures: each one moves an owned value in and consumes it.
macro_rules! fn_once_calls {
    ($name:ident, $guard:ident) => {
        #[inline(never)]
        fn $name(words: &[u32]) {
            let guard = $guard(3);
            let owned = $guard(words[1]);
            let once = #[inline(never)]
            move |i: usize| Pair {
                hi: owned.done() + words[i],
                lo: words[i + 1],
            };
            show_pair(once(2));
            let kept = $guard(words[2]);
            let once_none = #[inline(never)]
            move || kept.done() * 3;
            print_u32(once_none());
            let moved = $guard(words[3]);
            let once_three = #[inline(never)]
            move |a: u32, b: u64, c: u8| Wide {
                a: moved.done() + a,
                b,
                c,
                d: -1,
            };
            show_wide(once_three(5, 6, 7));
            print_u32(guard.done());
        }
    };
}

fn_calls!(fn_cleanup, Noisy);
fn_calls!(fn_plain, Plain);
fn_mut_calls!(fn_mut_cleanup, Noisy);
fn_mut_calls!(fn_mut_plain, Plain);
fn_once_calls!(fn_once_cleanup, Noisy);
fn_once_calls!(fn_once_plain, Plain);

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let words = [1u32, 2, 3, 4, 5, 6];
    print_u32(sum_lanes(&words[..4], 0));
    print_u32(sum_lanes(&words[..4], 1));
    print_str("fn");
    fn_cleanup(&words);
    fn_plain(&words);
    print_str("fn_mut");
    fn_mut_cleanup(&words);
    fn_mut_plain(&words);
    print_str("fn_once");
    fn_once_cleanup(&words);
    fn_once_plain(&words);
}
