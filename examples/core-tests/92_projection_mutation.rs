//! A copied call result must observe mutations performed after the call.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Pair {
    left: u32,
    right: u32,
}

impl Pair {
    #[inline(never)]
    fn new(left: u32, right: u32) -> Self {
        Self { left, right }
    }

    #[inline(never)]
    fn add(&mut self, other: Self) {
        self.left = self.left.wrapping_add(other.left);
        self.right = self.right.wrapping_add(other.right);
    }
}

#[inline(never)]
fn copy_after_mutation(seed: u32) -> Pair {
    let mut value = Pair::new(seed, seed.wrapping_add(1));
    value.add(Pair::new(8, 10));
    value
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let copied = copy_after_mutation(1);
    print_u32(copied.left);
    print_u32(copied.right);
}
