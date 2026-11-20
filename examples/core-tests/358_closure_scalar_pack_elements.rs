//! The element of a constant closure argument tuple whose layout is one scalar, at every kind of
//! type that can be that scalar (MIRES-020).
//!
//! A tuple is a single scalar when exactly one of its elements is not zero-sized and that element
//! is itself a scalar: `(bool,)`, `(char,)`, an integer or float of any width, a newtype, an enum
//! whose tag or niche is the whole value (`Ordering`, `Option<NonZero<u32>>`, `Option<char>`,
//! `Option<&u32>`), a raw pointer, or any of those next to zero-sized elements, as in
//! `(u32, (), PhantomData<u8>)`. With `-Copt-level=z` a call with such a constant tuple passes
//! it as that scalar, and the scalar has to come apart into the right value for each element.
//! Every caller here also holds a value with a destructor across the calls, so it cannot fall
//! back to the baseline lane.

#![no_std]
#![no_main]
#![feature(stmt_expr_attributes)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cmp::Ordering;
use core::marker::PhantomData;
use core::num::NonZero;

struct Noisy(u32);

impl Drop for Noisy {
    fn drop(&mut self) {
        print_u32(9000 + self.0);
    }
}

#[derive(Clone, Copy)]
struct Meters(u16);

#[repr(transparent)]
#[derive(Clone, Copy)]
struct Seconds(i32);

#[derive(Clone, Copy)]
enum Color {
    Red = 3,
    Green = 7,
    Blue = 200,
}

#[derive(Clone, Copy)]
struct Tagged {
    _marker: PhantomData<u8>,
    value: i8,
    _unit: (),
}

#[inline(never)]
fn integers(base: u32) {
    let _guard = Noisy(1);
    let by_u8 = #[inline(never)]
    |x: u8| base + x as u32;
    let by_i16 = #[inline(never)]
    |x: i16| base as i32 + x as i32;
    let by_u64 = #[inline(never)]
    |x: u64| x + base as u64;
    let by_i64 = #[inline(never)]
    |x: i64| x - base as i64;
    let by_u128 = #[inline(never)]
    |x: u128| (x >> 64) as u64 + base as u64;
    let by_usize = #[inline(never)]
    |x: usize| x * base as usize;
    print_u32(by_u8(250));
    print_i32(by_i16(-300));
    print_u64(by_u64(1 << 40));
    print_i64(by_i64(-(1 << 50)));
    print_u64(by_u128(7 << 64));
    print_usize(by_usize(6));
}

#[inline(never)]
fn primitives(base: u32) {
    let _guard = Noisy(2);
    let by_bool = #[inline(never)]
    |x: bool| if x { base } else { 0 };
    let by_char = #[inline(never)]
    |x: char| x as u32 + base;
    let by_f32 = #[inline(never)]
    |x: f32| (x * 4.0) as i32 + base as i32;
    let by_f64 = #[inline(never)]
    |x: f64| (x * 8.0) as i64 + base as i64;
    let by_ptr = #[inline(never)]
    |x: *const u8| x.is_null() as u32 + base;
    let by_ref = #[inline(never)]
    |x: &u32| *x + base;
    print_u32(by_bool(true));
    print_u32(by_bool(false));
    print_u32(by_char('Z'));
    print_u32(by_char('\u{1F600}'));
    print_i32(by_f32(-2.5));
    print_i64(by_f64(1.25));
    print_u32(by_ptr(core::ptr::null()));
    print_u32(by_ref(&40));
}

#[inline(never)]
fn wrappers(base: u32) {
    let _guard = Noisy(3);
    let by_newtype = #[inline(never)]
    |x: Meters| x.0 as u32 + base;
    let by_transparent = #[inline(never)]
    |x: Seconds| x.0 - base as i32;
    let by_padded = #[inline(never)]
    |x: Tagged| x.value as i32 * base as i32;
    print_u32(by_newtype(Meters(65535)));
    print_i32(by_transparent(Seconds(-5)));
    print_i32(by_padded(Tagged {
        _marker: PhantomData,
        value: -7,
        _unit: (),
    }));
}

#[inline(never)]
fn enums(base: u32) {
    let _guard = Noisy(4);
    let by_ordering = #[inline(never)]
    |x: Ordering| x as i32 * base as i32;
    let by_color = #[inline(never)]
    |x: Color| x as u32 + base;
    let by_nonzero = #[inline(never)]
    |x: Option<NonZero<u32>>| x.map_or(0, |n| n.get()) + base;
    let by_char_opt = #[inline(never)]
    |x: Option<char>| x.map_or(1, |c| c as u32) + base;
    let by_ref_opt = #[inline(never)]
    |x: Option<&u32>| x.map_or(2, |r| *r) + base;
    print_i32(by_ordering(Ordering::Less));
    print_i32(by_ordering(Ordering::Greater));
    print_u32(by_color(Color::Red));
    print_u32(by_color(Color::Green));
    print_u32(by_color(Color::Blue));
    print_u32(by_nonzero(NonZero::new(77)));
    print_u32(by_nonzero(None));
    print_u32(by_char_opt(Some('a')));
    print_u32(by_char_opt(None));
    print_u32(by_ref_opt(Some(&500)));
    print_u32(by_ref_opt(None));
}

#[inline(never)]
fn zero_sized_neighbours(base: u32) {
    let _guard = Noisy(5);
    let leading = #[inline(never)]
    |_: (), x: u32, _: PhantomData<u8>| x + base;
    let trailing = #[inline(never)]
    |x: u64, _: [u8; 0], _: ()| x * base as u64;
    let only_units = #[inline(never)]
    |_: (), _: PhantomData<u16>| base * 3;
    print_u32(leading((), 12, PhantomData));
    print_u64(trailing(1 << 35, [], ()));
    print_u32(only_units((), PhantomData));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    integers(10);
    primitives(20);
    wrappers(30);
    enums(40);
    zero_sized_neighbours(50);
}
