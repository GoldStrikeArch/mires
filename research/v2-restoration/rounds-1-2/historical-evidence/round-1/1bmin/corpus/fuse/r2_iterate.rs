//! Verifier fixture (r2_iterate): `for r in &v` collecting addresses and comparing with `&v[i]`; arrays and slices of references
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

plain_and_guarded!(iterate_values, iterate_values_guarded, 203, {
    let v = black_box([mk(1), mk(2), mk(3)]);
    let mut seen = [0usize; 3];
    let mut i = 0;
    for r in &v {
        seen[i] = r as *const P as usize;
        print_bool(ptr::eq(r, &v[i]));
        print_bool(r as *const P == &v[i] as *const P);
        i += 1;
    }
    print_bool(seen[0] == &v[0] as *const P as usize);
    print_bool(seen[1] == &v[1] as *const P as usize);
    print_bool(seen[2] == address(&v[2]));
    print_bool(seen[0] != seen[1] && seen[1] != seen[2] && seen[0] != seen[2]);
    for (k, r) in v.iter().enumerate() {
        print_bool(r as *const P as usize == seen[k]);
    }
    let s: &[P] = black_box(&v[..]);
    let mut j = 0;
    for r in s {
        print_bool(ptr::eq(r, &s[j]));
        print_bool(r as *const P as usize == seen[j]);
        j += 1;
    }
    print_u32(i as u32 + j as u32);
});

plain_and_guarded!(iterate_references, iterate_references_guarded, 204, {
    let v = black_box([mk(1), mk(2), mk(3)]);
    let refs: [&P; 3] = black_box([&v[2], &v[0], &v[1]]);
    let order = [2usize, 0, 1];
    let mut seen = [0usize; 3];
    let mut j = 0;
    for r in &refs {
        seen[j] = *r as *const P as usize;
        print_bool(ptr::eq(*r, &v[order[j]]));
        print_bool(r as *const &P as usize != *r as *const P as usize);
        j += 1;
    }
    print_bool(seen[0] == address(&v[2]));
    print_bool(seen[1] == address(&v[0]));
    print_bool(seen[2] == &v[1] as *const P as usize);
    let s: &[&P] = black_box(&refs[..]);
    for (k, r) in s.iter().enumerate() {
        print_bool(ptr::eq(*r, &v[order[k]]));
        print_bool(*r as *const P as usize == seen[k]);
    }
    let mut sum = 0;
    let mut k = 0;
    for r in refs {
        print_bool(r as *const P as usize == seen[k]);
        sum += unsafe { (*(r as *const P)).y };
        k += 1;
    }
    print_u32(sum);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    iterate_values();
    iterate_values_guarded();
    iterate_references();
    iterate_references_guarded();
}
