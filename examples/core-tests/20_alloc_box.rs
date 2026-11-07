//! `Box`: the first thing `alloc` gives a program, and the smallest allocation there is.
//!
//! A box is its pointer here (`CONTRACT.md`, "Value representation"), so `Box::new` is an
//! allocation, a retype of the fresh block to the element size, and one write; `*b` is the
//! ordinary dereference of a slot. What this pins is that all four shapes a box comes in behave:
//! a scalar, a struct written through, a trait object, and a boxed slice.
//!
//! Drop order is the last section, and it is the part a value model can most easily get wrong: a
//! box owns its allocation, so dropping one has to run the pointee's glue *and* free the block.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;

struct Point {
    x: i32,
    y: i32,
}

trait Shout {
    fn shout(&self) -> i32;
    fn twice(&self) -> i32 {
        self.shout() * 2
    }
}

struct Loud(i32);

impl Shout for Loud {
    fn shout(&self) -> i32 {
        self.0 + 1
    }
}

struct Noisy(i32);

impl Drop for Noisy {
    fn drop(&mut self) {
        print_i32(self.0);
    }
}

fn sum(values: &[i32]) -> i32 {
    let mut total = 0;
    for value in values {
        total += *value;
    }
    total
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // A box of a scalar: one element of four bytes.
    let scalar = Box::new(7i32);
    print_i32(*scalar);

    // A box of a struct, written through the box.
    let mut point = Box::new(Point { x: 1, y: 2 });
    point.x = 10;
    print_i32(point.x + point.y);

    // Moving out of a box, and moving one into a function.
    let owned = *Box::new(Point { x: 3, y: 4 });
    print_i32(owned.x * owned.y);

    // A trait object behind a box: an unsize coercion of the pointer, and a virtual call
    // through both an overridden method and a defaulted one.
    let shouter: Box<dyn Shout> = Box::new(Loud(20));
    print_i32(shouter.shout());
    print_i32(shouter.twice());

    // A boxed slice: indexing, length, and a borrow into an ordinary slice function.
    let values: Box<[i32]> = Box::new([4, 5, 6]);
    print_i32(values[1]);
    print_usize(values.len());
    print_i32(sum(&values));

    // Drop order: locals in reverse declaration order, then the boxed one where it is dropped.
    {
        let _first = Noisy(1);
        let _second = Noisy(2);
    }
    let boxed = Box::new(Noisy(3));
    drop(boxed);
    print_i32(99);
}
