//! A store an inlined `&mut` helper makes reaches the caller's struct.
//!
//! The reported shape: `fn set(s: &mut S, v: u8) { s.f = v }` inlined into a caller whose `a` was
//! built on two paths. The splice binds the callee's parameter to the caller's argument, which the
//! lowering spells `_7 = a; s = _7`: two more names for `a`'s object. The finished-JavaScript
//! scalar replacement used to read those assignments as copies and give each name its own field
//! locals, so `s.f = v` landed in a private copy and `a.f` printed its old value (7 for 254).
//! `130_reference_alias_store` is the same loss with no call at all.
//!
//! Every helper here has exactly one call site, which is what lets the inliner take it in a small
//! program: a free function, a `&mut self` method, an `Option<&mut S>` argument and a returned
//! `&mut S`, over `u8`, `u64`, `bool`, `f64` and a tuple newtype's `.0`, with plain and compound
//! stores. A helper the linker passes by value instead of inlining (`set_wide`, `scale` and `inc`
//! at the time of writing) still has to give the same answer.
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

fn set_byte(s: &mut Byte, v: u8) {
    s.f = v;
}

/// The original report: `set` is inlined, and its `s` is a second name for `a`.
fn byte_through_helper() {
    let mut a = Byte { f: black_box(1) };
    if black_box(true) {
        a = Byte { f: black_box(7) };
    }
    set_byte(&mut a, black_box(254));
    print_i32(a.f as i32);
}

#[derive(Clone, Copy)]
struct Wide {
    v: u64,
}

fn set_wide(s: &mut Wide, v: u64) {
    s.v = v;
}

fn wide_through_helper() {
    let mut a = Wide { v: black_box(1) };
    if black_box(true) {
        a = Wide { v: black_box(7) };
    }
    set_wide(&mut a, black_box(254));
    print_u64(a.v);
}

#[derive(Clone, Copy)]
struct Flag {
    on: bool,
}

fn turn_on(f: &mut Flag) {
    f.on = true;
}

fn flag_through_helper() {
    let mut f = Flag { on: false };
    if black_box(true) {
        f = Flag { on: black_box(false) };
    }
    turn_on(&mut f);
    print_bool(f.on);
}

#[derive(Clone, Copy)]
struct Count(u32);

fn inc(w: &mut Count) {
    w.0 += 1;
}

impl Count {
    fn add(&mut self, d: u32) {
        self.0 += d;
    }
}

/// A helper and a `&mut self` method on a tuple newtype, each with a compound store.
fn newtype_through_helper_and_method() {
    let mut w = if black_box(true) { Count(10) } else { Count(20) };
    inc(&mut w);
    print_u32(w.0);
    w.add(black_box(5));
    print_u32(w.0);
}

#[derive(Clone, Copy)]
struct Real {
    x: f64,
}

impl Real {
    fn scale(&mut self, k: f64) {
        self.x *= k;
    }
}

fn real_through_method() {
    let mut r = Real { x: black_box(1.5) };
    if black_box(true) {
        r = Real { x: black_box(7.5) };
    }
    r.scale(black_box(2.0));
    print_f64(r.x);
}

fn set_some(o: Option<&mut Byte>, v: u8) {
    if let Some(s) = o {
        s.f = v;
    }
}

/// The reference travels inside an `Option` into the helper.
fn byte_through_option() {
    let mut a = Byte { f: black_box(1) };
    if black_box(true) {
        a = Byte { f: black_box(7) };
    }
    set_some(Some(&mut a), black_box(254));
    print_i32(a.f as i32);
}

fn pick(s: &mut Byte) -> &mut Byte {
    s
}

/// The reference a helper returns is written by the caller.
fn byte_through_returned_reference() {
    let mut a = Byte { f: black_box(1) };
    if black_box(true) {
        a = Byte { f: black_box(7) };
    }
    pick(&mut a).f = black_box(254);
    print_i32(a.f as i32);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    byte_through_helper();
    wide_through_helper();
    flag_through_helper();
    newtype_through_helper_and_method();
    real_through_method();
    byte_through_option();
    byte_through_returned_reference();
}
