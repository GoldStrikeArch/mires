//! `core::cmp`: `Ordering`, the derived and hand-written comparison traits, `min`/`max`/`clamp`.

#![no_std]
#![no_main]

use core::cmp::{self, Ordering};

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
struct Version {
    major: u32,
    minor: u32,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum Level {
    Low,
    Mid,
    High,
}

/// A hand-written `Ord` that reverses the natural one, to prove the trait is dispatched rather
/// than the derive being special-cased.
#[derive(PartialEq, Eq, Clone, Copy)]
struct Backwards(i32);

impl PartialOrd for Backwards {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Backwards {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.cmp(&self.0)
    }
}

fn largest<T: Ord + Copy>(items: &[T; 4]) -> T {
    let mut best = items[0];
    let mut i = 1;
    while i < 4 {
        if items[i] > best {
            best = items[i];
        }
        i += 1;
    }
    best
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // `Ordering` on primitives.
    print_ordering(1i32.cmp(&2));
    print_ordering(2i32.cmp(&2));
    print_ordering(3i32.cmp(&2));
    print_ordering(1u64.cmp(&9));
    print_ordering('a'.cmp(&'b'));
    print_ordering(false.cmp(&true));

    // `Ordering`'s own methods.
    print_bool(Ordering::Less.is_lt());
    print_ordering(Ordering::Less.reverse());
    print_ordering(Ordering::Equal.then(Ordering::Greater));

    // min / max / clamp. `clamp` asserts `min <= max` through `const_panic!`, whose runtime
    // message the sysroot patch reduces to a plain string — see patches/.
    print_i32(cmp::min(3, 7));
    print_i32(cmp::max(3, 7));
    print_i32(3i32.min(9).max(-1));
    print_u32(7u32.min(2));
    print_i32(5i32.clamp(0, 4));
    print_i32((-5i32).clamp(0, 4));

    // Derived orderings compare lexicographically by field, then by variant.
    let a = Version { major: 1, minor: 9 };
    let b = Version { major: 2, minor: 0 };
    print_bool(a < b);
    print_bool(a == a);
    print_ordering(a.cmp(&b));
    print_ordering(b.cmp(&a));
    print_ordering(Level::Mid.cmp(&Level::High));
    print_bool(Level::High > Level::Low);

    // A hand-written `Ord`.
    print_ordering(Backwards(1).cmp(&Backwards(2)));
    print_bool(Backwards(1) > Backwards(2));

    // A generic function over `Ord`, instantiated twice.
    print_i32(largest(&[3, 9, 2, 7]));
    print_u32(largest(&[10u32, 4, 55, 6]));
}
