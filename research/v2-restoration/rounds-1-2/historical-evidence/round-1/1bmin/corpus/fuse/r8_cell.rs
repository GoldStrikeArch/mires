//! Verifier fixture (r8_cell): a boxed cell holding a reference (the case fixture 116 dropped)
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

struct R<'a> {
    n: u32,
    r: &'a P,
}

struct Guard(u32);

impl Drop for Guard {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

#[inline(never)]
fn mk(x: u32) -> P {
    black_box(P { x, y: x + 1 })
}

#[inline(never)]
fn address(p: *const P) -> usize {
    p as usize
}

macro_rules! plain_and_guarded {
    ($plain:ident, $guarded:ident, $tag:expr, $body:block) => {
        #[inline(never)]
        fn $plain() $body

        #[inline(never)]
        fn $guarded() {
            let _guard = Guard($tag);
            $body
        }
    };
}

plain_and_guarded!(cell_reference, cell_reference_guarded, 211, {
    let h = mk(1);
    let h2 = mk(2);
    let mut local: &P = &h;
    let slot: &mut &P = black_box(&mut local);
    print_bool(ptr::eq(*slot, &h));
    *slot = &h2;
    print_bool(ptr::eq(local, &h2));
    print_bool(local as *const P as usize == address(&h2));
    print_bool(local as *const P == &h2 as *const P);
    print_bool(!ptr::eq(local, &h));
    print_bool(&local as *const &P as usize != local as *const P as usize);
    print_u32(local.x);
});

plain_and_guarded!(cell_before_write, cell_before_write_guarded, 212, {
    let h = mk(1);
    let h2 = mk(2);
    let mut local: &P = &h;
    print_bool(ptr::eq(local, &h));
    print_bool(local as *const P as usize == address(&h));
    let slot: &mut &P = black_box(&mut local);
    print_bool(*slot as *const P as usize == address(&h));
    *slot = &h2;
    print_bool(*slot as *const P as usize == address(&h2));
    print_u32(local.y);
});

plain_and_guarded!(cell_option, cell_option_guarded, 213, {
    let h = mk(1);
    let h2 = mk(2);
    let mut local: Option<&P> = Some(&h);
    let slot: &mut Option<&P> = black_box(&mut local);
    print_bool(ptr::eq(slot.unwrap(), &h));
    *slot = Some(&h2);
    print_bool(ptr::eq(local.unwrap(), &h2));
    print_bool(local.unwrap() as *const P as usize == address(&h2));
    print_bool(local.map(|r| r as *const P) == Some(&h2 as *const P));
    print_u32(local.unwrap().x);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    cell_reference();
    cell_reference_guarded();
    cell_before_write();
    cell_before_write_guarded();
    cell_option();
    cell_option_guarded();
}
