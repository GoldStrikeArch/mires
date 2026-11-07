//! `core::mem`: the layout queries, the move helpers, and `Drop`.

#![no_std]
#![no_main]

use core::mem;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct Noisy(i32);

impl Drop for Noisy {
    fn drop(&mut self) {
        print_i32(self.0);
    }
}

struct Pair {
    first: Noisy,
    second: Noisy,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Point {
    x: i32,
    y: i32,
}

fn early(flag: bool) -> i32 {
    let _guard = Noisy(80);
    if flag {
        return 1;
    }
    let _second = Noisy(81);
    2
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Layout queries. The target is `wasm32-unknown-unknown`, so a pointer is four bytes.
    print_usize(mem::size_of::<i32>());
    print_usize(mem::size_of::<u64>());
    print_usize(mem::size_of::<bool>());
    print_usize(mem::size_of::<()>());
    print_usize(mem::size_of::<Point>());
    print_usize(mem::size_of::<&i32>());
    print_usize(mem::size_of::<Option<i32>>());
    print_usize(mem::align_of::<u64>());
    print_usize(mem::align_of::<u8>());

    // `replace` and `take` return the old value and leave a new one behind.
    let mut value = 10i32;
    print_i32(mem::replace(&mut value, 20));
    print_i32(value);
    print_i32(mem::take(&mut value));
    print_i32(value);

    let mut point = Point { x: 1, y: 2 };
    let old = mem::replace(&mut point, Point { x: 3, y: 4 });
    print_i32(old.x + old.y);
    print_i32(point.x + point.y);
    print_bool(mem::take(&mut point) == Point { x: 3, y: 4 });
    print_bool(point == Point::default());

    // `swap` exchanges two places.
    let mut a = 1i32;
    let mut b = 2i32;
    mem::swap(&mut a, &mut b);
    print_i32(a);
    print_i32(b);

    let mut left = Point { x: 5, y: 6 };
    let mut right = Point { x: 7, y: 8 };
    mem::swap(&mut left, &mut right);
    print_i32(left.x);
    print_i32(right.x);

    // Drop order: locals drop in reverse declaration order, fields in declaration order.
    print_i32(0);
    {
        let _a = Noisy(1);
        let _b = Noisy(2);
    }
    {
        let _pair = Pair { first: Noisy(3), second: Noisy(4) };
    }

    // `mem::drop` runs the destructor early; `mem::forget` skips it entirely.
    let early_drop = Noisy(5);
    drop(early_drop);
    print_i32(6);
    mem::forget(Noisy(7));
    print_i32(8);

    // `ManuallyDrop` also suppresses the destructor.
    let _kept = mem::ManuallyDrop::new(Noisy(9));
    print_i32(10);

    // Drops on an early return.
    print_i32(early(true));
    print_i32(early(false));

    // A moved-out value is dropped by its new owner, once.
    let moved = Noisy(11);
    let holder = Pair { first: moved, second: Noisy(12) };
    print_i32(13);
    drop(holder);
    print_i32(14);
}
