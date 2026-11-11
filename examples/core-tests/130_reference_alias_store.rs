//! A store through a reference picked on two paths reaches the struct it names.
//!
//! A `&mut S` is the struct's own JavaScript object, so `let p = if c { &mut a } else { &mut b }`
//! lowers to `p = a` on one path and `p = b` on the other: a second name for one object, never a
//! copy. The finished-JavaScript scalar replacement used to read every such whole-local assignment
//! as a copy, give `a`, `b` and `p` one local per field each, and so drop `p.f = 254` on the floor:
//! `a.f` still printed its old value. No call is involved, so this is not an inlining artifact; an
//! inlined `&mut` helper (`131_inlined_mut_helper_store`) is the same shape with the parameter as
//! the second name.
//!
//! Each body builds its struct on two paths (which keeps the alias from being forwarded to a
//! once-written root), picks a reference by `if`, by `match`, by a loop that moves a cursor, or by
//! a reborrow, and writes one field: a `u8`, `u32` fields with `+=`, a `u64`, a `bool`, an `f64`
//! with `*=`, a tuple newtype's `.0`, and a non-`Copy` struct moved in. Reads come before, between
//! and after the stores, through the struct and through a shared reference. `copies_stay_copies`
//! is the other direction: a by-value copy that is then changed must stay independent of its
//! source.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

#[derive(Clone, Copy)]
struct Byte {
    f: u8,
}

#[derive(Clone, Copy)]
struct Point {
    x: u32,
    y: u32,
}

#[derive(Clone, Copy)]
struct Wide {
    v: u64,
}

#[derive(Clone, Copy)]
struct Flag {
    on: bool,
}

#[derive(Clone, Copy)]
struct Real {
    x: f64,
}

#[derive(Clone, Copy)]
struct Count(u16);

struct Owned {
    f: u8,
}

/// `p` names `a` or `b`, picked on two paths; `a` was itself built on two paths.
fn chosen_by_if() {
    let mut a = Byte { f: black_box(1) };
    let mut b = Byte { f: black_box(2) };
    if black_box(true) {
        a = Byte { f: black_box(7) };
    }
    print_i32(a.f as i32);
    let p: &mut Byte = if black_box(true) { &mut a } else { &mut b };
    p.f = black_box(254);
    print_i32(a.f as i32);
    print_i32(b.f as i32);
}

/// The same choice by `match`, then one compound and one plain store.
fn chosen_by_match() {
    let mut a = Point { x: black_box(1), y: 2 };
    let mut b = Point { x: black_box(3), y: 4 };
    if black_box(false) {
        b = Point { x: 9, y: 9 };
    }
    let p = match black_box(1u8) {
        0 => &mut a,
        _ => &mut b,
    };
    p.x += black_box(100);
    print_u32(p.x);
    p.y = 50;
    print_u32(a.x);
    print_u32(a.y);
    print_u32(b.x);
    print_u32(b.y);
}

/// A cursor that moves from one struct to the next inside a loop.
fn loop_cursor() {
    let mut a = Byte { f: black_box(1) };
    let mut b = Byte { f: black_box(2) };
    if black_box(true) {
        a = Byte { f: 10 };
        b = Byte { f: 20 };
    }
    let mut p = &mut a;
    let mut i = 0;
    while i < black_box(3) {
        if i == 1 {
            p = &mut b;
        }
        p.f += 1;
        i += 1;
    }
    print_i32(a.f as i32);
    print_i32(b.f as i32);
}

/// A reborrow of one reference against a borrow of another struct.
fn reborrow_or_other() {
    let mut a = Byte { f: black_box(1) };
    let mut b = Byte { f: black_box(2) };
    if black_box(true) {
        a = Byte { f: black_box(7) };
    }
    let p = &mut a;
    let q = if black_box(true) { &mut *p } else { &mut b };
    q.f = black_box(254);
    print_i32(a.f as i32);
    print_i32(b.f as i32);
}

/// Wider and non-integer fields, and a tuple newtype, through the same two-path reference.
fn other_field_types() {
    let mut w = Wide { v: black_box(1) };
    let mut w2 = Wide { v: black_box(2) };
    if black_box(true) {
        w = Wide { v: black_box(7) };
    }
    let p = if black_box(true) { &mut w } else { &mut w2 };
    p.v = black_box(254);
    print_u64(w.v);

    let mut f = Flag { on: black_box(false) };
    let mut f2 = Flag { on: black_box(false) };
    if black_box(true) {
        f = Flag { on: black_box(false) };
    }
    let p = if black_box(true) { &mut f } else { &mut f2 };
    p.on = true;
    print_bool(f.on);

    let mut r = Real { x: black_box(1.5) };
    let mut r2 = Real { x: black_box(2.5) };
    if black_box(true) {
        r = Real { x: black_box(7.5) };
    }
    let p = if black_box(true) { &mut r } else { &mut r2 };
    p.x *= 2.0;
    print_f64(r.x);

    let mut c = Count(black_box(10));
    let mut c2 = Count(black_box(20));
    if black_box(false) {
        c = Count(30);
    }
    let p = if black_box(true) { &mut c } else { &mut c2 };
    p.0 += 6;
    print_u32(c.0 as u32);
}

/// A non-`Copy` struct moved in on two paths and written through a reference.
fn owned_moved_in() {
    let a = Owned { f: black_box(1) };
    let mut b = if black_box(true) { a } else { Owned { f: 3 } };
    let mut c = Owned { f: black_box(4) };
    let p = if black_box(true) { &mut b } else { &mut c };
    p.f = black_box(254);
    print_i32(b.f as i32);
    print_i32(c.f as i32);
}

/// Shared readers before and after one writer.
fn readers_around_a_writer() {
    let mut a = Point { x: black_box(1), y: 2 };
    let mut b = Point { x: black_box(3), y: 4 };
    if black_box(true) {
        a = Point { x: black_box(7), y: 8 };
    }
    let r = if black_box(true) { &a } else { &b };
    print_u32(r.x);
    let w = if black_box(true) { &mut a } else { &mut b };
    w.x = black_box(254);
    let r = if black_box(true) { &a } else { &b };
    print_u32(r.x);
    print_u32(a.x + a.y);
}

/// The other direction must stay a copy: a by-value copy that is then changed does not change
/// its source, and a source changed later does not change the copy.
fn copies_stay_copies() {
    let mut a = Byte { f: black_box(1) };
    if black_box(true) {
        a = Byte { f: black_box(7) };
    }
    let mut b = a;
    b.f = black_box(254);
    print_i32(a.f as i32);
    print_i32(b.f as i32);
    let c = a;
    a.f = black_box(9);
    print_i32(a.f as i32);
    print_i32(c.f as i32);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    chosen_by_if();
    chosen_by_match();
    loop_cursor();
    reborrow_or_other();
    other_field_types();
    owned_moved_in();
    readers_around_a_writer();
    copies_stay_copies();
}
