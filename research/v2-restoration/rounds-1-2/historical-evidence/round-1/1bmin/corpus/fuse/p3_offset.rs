//! Verifier fixture (p3_offset): element-reference pointer arithmetic: offset_from, add, and a write through a pointer made from `&mut v[i]`
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

#[inline(never)]
fn elem_offset(v: &[P], i: usize, j: usize) -> isize {
    unsafe { (&v[j] as *const P).offset_from(&v[i] as *const P) }
}

#[inline(never)]
fn next_x(v: &[P], i: usize) -> u32 {
    let p = &v[i] as *const P;
    unsafe { (*p.add(1)).x }
}

#[inline(never)]
fn write_elem(v: &mut [P], i: usize, x: u32) {
    let p = &mut v[i] as *mut P;
    unsafe { (*p).x = x };
}

#[inline(never)]
fn replace_elem(v: &mut [P], i: usize, n: P) -> P {
    core::mem::replace(&mut v[i], n)
}

plain_and_guarded!(arithmetic, arithmetic_guarded, 406, {
    let mut v = black_box([mk(1), mk(2), mk(3)]);
    write_elem(&mut v, 1, 20);
    print_u32(v[1].x);
    let old = replace_elem(&mut v, 2, mk(30));
    print_u32(old.x + v[2].x);
    print_bool(elem_offset(&v, 0, 2) == 2);
    let p0 = &v[0] as *const P;
    let p2 = &v[2] as *const P;
    print_bool(unsafe { p2.offset_from(p0) } == 2);
    print_bool(p0 < p2);
    print_bool(p2 as usize - p0 as usize == 2 * core::mem::size_of::<P>());
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    arithmetic();
    arithmetic_guarded();
}
