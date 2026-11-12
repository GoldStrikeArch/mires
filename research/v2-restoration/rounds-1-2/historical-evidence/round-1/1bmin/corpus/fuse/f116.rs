//! A reference read out of a place is a pointer *value*: made into a raw pointer, compared with
//! `ptr::eq`/`addr_eq` or cast to `usize`, it is the address it holds, never the address of the
//! field or element it was read from.
//!
//! A reference to an aggregate is the object itself, and the raw pointer made from it is that
//! object's one `__rt.box` slot, so every route to the same object yields one pointer. The fusion
//! planner may spell a loaded reference inline as the read itself (`a.r`, `refs[i]`, `xs[i]`,
//! `pp.buf[pp.off]`), and the raw-pointer conversion must still see a value there, not the slot
//! of the place the value came from. Every shape is asked twice: in a body with no
//! cleanup edge, and beside a live `Drop` guard, which gives every later call an unwind edge. With
//! `js-cleanup=on` the two take the two fusion paths (`decide_inline` and the adjacent-view path of
//! a body with an `Invoke`); with `js-cleanup=off` both take the first. Only relations Rust defines
//! are printed: distinct live objects have distinct addresses.

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

struct R2<'a> {
    r: &'a P,
    s: &'a P,
}

struct RO<'a> {
    n: u32,
    o: Option<&'a P>,
}

struct RM<'a> {
    n: u32,
    r: &'a mut P,
}

/// Prints its tag when dropped, so everything after it in a body has a cleanup edge.
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

/// Defines `$plain` with `$body`, and `$guarded` with the same body behind a live `Guard($tag)`.
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

// A `&P` field compared with the place it points at, by every comparison Rust has, and the field's
// own address told apart from the address it holds.
plain_and_guarded!(field_pointer, field_pointer_guarded, 100, {
    let h = mk(1);
    let h2 = mk(2);
    let a = black_box(R { n: 1, r: &h });
    print_bool(ptr::eq(a.r, &h));
    print_bool(!ptr::eq(a.r, &h2));
    print_bool(ptr::addr_eq(a.r, &h));
    print_bool(a.r as *const P == &h as *const P);
    print_bool(a.r as *const P as usize == &h as *const P as usize);
    print_bool(&a.r as *const &P as usize != a.r as *const P as usize);
    print_bool(address(a.r) == address(&h));
    print_u32(unsafe { (*(a.r as *const P)).y } + a.n);
});

// Two reference fields, equal and different.
plain_and_guarded!(two_fields, two_fields_guarded, 101, {
    let h = mk(1);
    let h2 = mk(2);
    let a = black_box(R2 { r: &h, s: &h2 });
    print_bool(!ptr::eq(a.r, a.s));
    print_bool(a.r as *const P as usize != a.s as *const P as usize);
    let b = black_box(R2 { r: &h2, s: &h2 });
    print_bool(ptr::eq(b.r, b.s));
    print_bool(ptr::eq(b.r, a.s));
    print_bool(b.s as *const P as usize == a.s as *const P as usize);
});

// An `Option<&P>` field: unwrapped, matched, mapped to a raw pointer, reassigned and cleared.
plain_and_guarded!(option_field, option_field_guarded, 102, {
    let h = mk(1);
    let h2 = mk(2);
    let mut a = black_box(RO { n: 1, o: Some(&h) });
    print_bool(ptr::eq(a.o.unwrap(), &h));
    if let Some(r) = a.o {
        print_bool(r as *const P as usize == &h as *const P as usize);
    }
    print_bool(a.o.map(|r| r as *const P) == Some(&h as *const P));
    let before = a.o.unwrap() as *const P as usize;
    a.o = black_box(Some(&h2));
    print_bool(ptr::eq(a.o.unwrap(), &h2));
    print_bool(a.o.unwrap() as *const P as usize != before);
    print_bool(&a.o as *const Option<&P> as usize != a.o.unwrap() as *const P as usize);
    print_u32(a.o.unwrap().x + a.n);
    a.o = black_box(None);
    print_bool(a.o.is_none());
});

// A `&mut P` field written through as a raw pointer, retargeted, and written through again.
plain_and_guarded!(mut_field, mut_field_guarded, 103, {
    let mut h = mk(1);
    let mut h2 = mk(2);
    let h_at = &h as *const P as usize;
    let h2_at = &h2 as *const P as usize;
    {
        let mut a = black_box(RM { n: 1, r: &mut h });
        let p: *mut P = a.r;
        print_bool(p as usize == h_at);
        unsafe { (*p).x = 9 };
        print_u32(a.r.x);
        a.r = black_box(&mut h2);
        print_bool(a.r as *mut P as usize == h2_at);
        print_bool(a.r as *const P as usize != h_at);
        a.r.y = 7;
        print_u32(a.r.y + a.n);
    }
    print_u32(h.x);
    print_u32(h.y);
    print_u32(h2.y);
});

// A reference to an array element compared with a fresh borrow of the element, and an array of
// references read by constant and by computed index, before and after an element is replaced.
plain_and_guarded!(element_refs, element_refs_guarded, 104, {
    let a = black_box([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]);
    let r = black_box(&a[1]);
    print_bool(ptr::eq(r, &a[1]));
    print_bool(ptr::addr_eq(r, &a[1]));
    print_bool(r as *const P as usize == &a[1] as *const P as usize);
    print_bool(!ptr::eq(r, &a[black_box(2)]));
    print_bool(ptr::eq(r, &a[black_box(1)]));
    print_u32(r.x);
    let h = mk(1);
    let h2 = mk(2);
    let mut refs: [&P; 2] = black_box([&h, &h2]);
    print_bool(ptr::eq(refs[0], &h));
    print_bool(refs[1] as *const P as usize == &h2 as *const P as usize);
    print_bool(ptr::eq(refs[black_box(1)], &h2));
    print_bool(&refs[0] as *const &P as usize != refs[0] as *const P as usize);
    refs[0] = black_box(&h2);
    print_bool(ptr::eq(refs[0], refs[1]));
    print_bool(!ptr::eq(refs[0], &h));
    let mut seen = 0;
    for r in &refs {
        if ptr::eq(*r, &h2) {
            seen += 1;
        }
    }
    print_u32(seen);
});

// A reference read back through a reference to it, and one read through a `&mut &P` to a local.
// (The raw pointer of that local's own read after the write is not asked: the baseline lane
// answers the local's slot there, a gap outside fusion.)
plain_and_guarded!(behind_reference, behind_reference_guarded, 105, {
    let h = mk(1);
    let h2 = mk(2);
    let mut a = black_box(R { n: 1, r: &h });
    let pp: &&P = &a.r;
    print_bool(ptr::eq(*pp, &h));
    print_bool(*pp as *const P as usize == &h as *const P as usize);
    print_bool(pp as *const &P as usize != *pp as *const P as usize);
    a.r = black_box(&h2);
    let pp: &&P = black_box(&a.r);
    print_bool(ptr::eq(*pp, &h2));
    let mut local: &P = &h;
    let slot: &mut &P = black_box(&mut local);
    print_bool(ptr::eq(*slot, &h));
    *slot = &h2;
    print_u32(local.x + a.n);
});

// The pointer taken from a field before the field is reassigned keeps pointing at the old place;
// the field reads the new one.
plain_and_guarded!(read_after_reassign, read_after_reassign_guarded, 106, {
    let h = mk(1);
    let h2 = mk(2);
    let mut a = black_box(R { n: 1, r: &h });
    let p = a.r as *const P;
    a.r = black_box(&h2);
    print_u32(unsafe { (*p).x });
    print_u32(a.r.x);
    print_bool(ptr::eq(p, &h));
    print_bool(ptr::eq(a.r, &h2));
    print_bool(!ptr::eq(p, a.r));
    let mut dst = mk(0);
    unsafe { ptr::copy_nonoverlapping(a.r as *const P, &mut dst, 1) };
    print_u32(dst.x + dst.y);
    let read = unsafe { ptr::read(p) };
    print_u32(read.y);
});

#[unsafe(no_mangle)]
pub fn rust_entry() {
    field_pointer();
    field_pointer_guarded();
    two_fields();
    two_fields_guarded();
    option_field();
    option_field_guarded();
    mut_field();
    mut_field_guarded();
    element_refs();
    element_refs_guarded();
    behind_reference();
    behind_reference_guarded();
    read_after_reassign();
    read_after_reassign_guarded();
}
