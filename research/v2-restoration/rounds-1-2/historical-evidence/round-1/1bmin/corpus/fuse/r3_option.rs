//! Verifier fixture (r3_option): `Option<&P>::map(|r| r as *const P)` and the other Option adapters
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

struct RO<'a> {
    n: u32,
    o: Option<&'a P>,
}

plain_and_guarded!(option_map, option_map_guarded, 205, {
    let h = mk(1);
    let h2 = mk(2);
    let o: Option<&P> = black_box(Some(&h));
    let p = o.map(|r| r as *const P);
    print_bool(p == Some(&h as *const P));
    print_bool(p.map(|p| p as usize) == Some(address(&h)));
    print_bool(p != Some(&h2 as *const P));
    let a = black_box(RO { n: 1, o: Some(&h2) });
    print_bool(a.o.map(|r| r as *const P) == Some(&h2 as *const P));
    print_bool(a.o.map(|r| r as *const P as usize) == Some(address(&h2)));
    print_bool(a.o.map(|r| ptr::eq(r, &h2)) == Some(true));
    print_bool(a.o.map(ptr::from_ref) == Some(&raw const h2));
    print_bool(a.o.map(|r| r as *const P) != p);
    if let Some(r) = &a.o {
        print_bool(ptr::eq(*r, &h2));
        print_bool(r as *const &P as usize != *r as *const P as usize);
    }
    print_bool(ptr::eq(*a.o.as_ref().unwrap(), &h2));
    print_bool(a.o.map_or(0, |r| r as *const P as usize) == address(&h2));
    let none: Option<&P> = black_box(None);
    print_bool(none.map(|r| r as *const P).is_none());
    print_u32(a.n + unsafe { (*p.unwrap()).x });
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    option_map();
    option_map_guarded();
}
