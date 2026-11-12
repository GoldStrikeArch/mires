#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code, invalid_value)]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;
use core::hint::black_box;
use core::num::{NonZeroU8, NonZeroU32, NonZeroU64};

#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi2 { A = 200, B = 254 }
// niche values 255, 0: all-zero bytes are R
#[derive(Clone, Copy)] enum O2 { P(Hi2), Q, R }
#[derive(Clone, Copy)] enum OArr { P([Hi2; 2]), Q, R }
#[derive(Clone, Copy)] enum OTupS { P((SL<Hi2>, ())), Q, R }

static SEVEN: u32 = 7;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // safe: `vec![elem; n]` of an all-zero element allocates zeroed
    let a: Vec<Option<&'static u32>> = vec![None; black_box(3)];
    print_usize(a.iter().filter(|o| o.is_none()).count());
    let b: Vec<Option<NonZeroU8>> = vec![None; black_box(3)];
    print_usize(b.iter().filter(|o| o.is_none()).count());
    let c: Vec<Option<NonZeroU64>> = vec![None; black_box(2)];
    print_usize(c.iter().filter(|o| o.is_none()).count());
    let e: Vec<(Option<&'static u32>, u8)> = vec![(None, 0); black_box(2)];
    print_usize(e.iter().filter(|o| o.0.is_none() && o.1 == 0).count());
    let mut g: Vec<Option<&'static u32>> = vec![None; black_box(2)];
    g[1] = Some(&SEVEN);
    print_u32(g[1].map_or(0, |v| *v)); print_bool(g[0].is_none());
    let h: Vec<Option<char>> = vec![None; black_box(2)];
    print_usize(h.iter().filter(|o| o.is_none()).count());
    let i: Vec<Option<SL<NonZeroU8>>> = vec![None; black_box(2)];
    print_usize(i.iter().filter(|o| o.is_none()).count());
    // unsafe, every value in its normal representation: zeroed heap slices
    let z1: Box<[Option<SL<char>>]> = unsafe { Box::<[Option<SL<char>>]>::new_zeroed_slice(black_box(2)).assume_init() };
    print_u32(match z1[1] { Some(SL::Live(c)) => 100 + c as u32, Some(_) => 99, None => 1 });
    let z2: Box<[Option<[bool; 2]>]> = unsafe { Box::<[Option<[bool; 2]>]>::new_zeroed_slice(black_box(2)).assume_init() };
    print_u32(match z2[1] { Some(b) => 100 + b[0] as u32 + b[1] as u32, None => 1 });
    let z3: Box<[O2]> = unsafe { Box::<[O2]>::new_zeroed_slice(black_box(2)).assume_init() };
    print_u32(match z3[1] { O2::P(h) => h as u32, O2::Q => 1, O2::R => 2 });
    let z4: Box<[OArr]> = unsafe { Box::<[OArr]>::new_zeroed_slice(black_box(2)).assume_init() };
    print_u32(match z4[1] { OArr::P(a) => a[0] as u32, OArr::Q => 1, OArr::R => 2 });
    let z5: Box<[OTupS]> = unsafe { Box::<[OTupS]>::new_zeroed_slice(black_box(2)).assume_init() };
    print_u32(match z5[1] { OTupS::P(_) => 99, OTupS::Q => 1, OTupS::R => 2 });
    let z6: Box<Option<NonZeroU32>> = unsafe { Box::<Option<NonZeroU32>>::new_zeroed().assume_init() };
    print_bool(z6.is_none());
    let z7: Box<O2> = unsafe { Box::<O2>::new_zeroed().assume_init() };
    print_u32(match *z7 { O2::P(h) => h as u32, O2::Q => 1, O2::R => 2 });
}
