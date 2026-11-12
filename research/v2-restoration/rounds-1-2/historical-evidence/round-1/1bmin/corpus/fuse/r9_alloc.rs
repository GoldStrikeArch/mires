//! Verifier fixture (r9_alloc): references in a Vec, behind a Box, and collected addresses
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

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

plain_and_guarded!(vec_of_refs, vec_of_refs_guarded, 214, {
    let h = mk(1);
    let h2 = mk(2);
    let h3 = mk(3);
    let v: Vec<&P> = vec![&h, &h2, &h3];
    let addrs: Vec<usize> = v.iter().map(|r| *r as *const P as usize).collect();
    print_bool(addrs[0] == address(&h));
    print_bool(addrs[1] == address(&h2));
    print_bool(addrs[2] == &h3 as *const P as usize);
    print_bool(ptr::eq(v[1], &h2));
    print_bool(v[black_box(2)] as *const P as usize == addrs[2]);
    let mut n = 0;
    for r in &v {
        if ptr::eq(*r, &h2) {
            n += 1;
        }
    }
    print_u32(n);
    print_bool(v.iter().position(|r| ptr::eq(*r, &h3)) == Some(2));
    print_bool(v.first().map(|r| *r as *const P) == Some(&h as *const P));
    let b: Box<R> = Box::new(black_box(R { n: 1, r: &h2 }));
    print_bool(ptr::eq(b.r, &h2));
    print_bool(b.r as *const P as usize == addrs[1]);
    let bb: Box<&P> = Box::new(&h3);
    print_bool(ptr::eq(*bb, &h3));
    print_bool(*bb as *const P as usize == addrs[2]);
    let owned: Vec<P> = vec![mk(4), mk(5)];
    let mut at = [0usize; 2];
    let mut i = 0;
    for r in &owned {
        at[i] = r as *const P as usize;
        i += 1;
    }
    print_bool(at[0] == &owned[0] as *const P as usize);
    print_bool(at[1] == &owned[1] as *const P as usize);
    print_bool(at[0] != at[1]);
    print_u32(b.n + owned[1].x);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    vec_of_refs();
    vec_of_refs_guarded();
}
