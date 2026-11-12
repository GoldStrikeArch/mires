//! Verifier fixture (r4_nonnull): `NonNull::from(a.r)`, of the field holding it, and of an element
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

use core::ptr::NonNull;

plain_and_guarded!(nonnull, nonnull_guarded, 206, {
    let h = mk(1);
    let h2 = mk(2);
    let a = black_box(R { n: 1, r: &h });
    let nn: NonNull<P> = NonNull::from(a.r);
    print_bool(nn.as_ptr() as *const P == &h as *const P);
    print_bool(ptr::eq(nn.as_ptr(), &h));
    print_bool(nn.as_ptr() as usize == address(&h));
    print_bool(nn == NonNull::from(&h));
    print_u32(unsafe { nn.as_ref().x });
    let nn2: NonNull<&P> = NonNull::from(&a.r);
    print_bool(nn2.as_ptr() as usize != nn.as_ptr() as usize);
    print_bool(ptr::eq(unsafe { *nn2.as_ptr() }, &h));
    print_bool(unsafe { *nn2.as_ref() } as *const P as usize == address(&h));
    let nn3 = NonNull::from(&h2);
    print_bool(!ptr::eq(nn.as_ptr(), nn3.as_ptr()));
    print_bool(nn != nn3);
    print_bool(ptr::eq(nn3.as_ptr(), &h2));
    let refs: [&P; 2] = black_box([&h, &h2]);
    print_bool(ptr::eq(NonNull::from(refs[1]).as_ptr(), &h2));
    print_bool(NonNull::from(refs[black_box(0)]).as_ptr() as usize == address(&h));
    print_bool(NonNull::from(refs[1]) == nn3);
    print_u32(a.n);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    nonnull();
    nonnull_guarded();
}
