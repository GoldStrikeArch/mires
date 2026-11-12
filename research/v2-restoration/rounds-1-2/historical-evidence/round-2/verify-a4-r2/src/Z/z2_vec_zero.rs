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
use core::mem::MaybeUninit;
use core::num::{NonZeroU8, NonZeroU32, NonZeroU64};

#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi2 { A = 200, B = 254 }
// the niche sits in the first array element; values 255, 0: zero is R
#[derive(Clone, Copy)] enum OArr { P([Hi2; 2]), Q, R }
#[derive(Clone, Copy)] enum OArrS { P([SL<Hi2>; 3]), Q, R }
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
    let d: Vec<[Option<NonZeroU32>; 2]> = vec![[None; 2]; black_box(2)];
    print_usize(d.iter().flatten().filter(|o| o.is_none()).count());
    let e: Vec<(Option<&'static u32>, u8)> = vec![(None, 0); black_box(2)];
    print_usize(e.iter().filter(|o| o.0.is_none() && o.1 == 0).count());
    let f: Vec<Option<Box<u32>>> = (0..black_box(3)).map(|_| None).collect();
    print_usize(f.iter().filter(|o| o.is_none()).count());
    let mut g: Vec<Option<&'static u32>> = vec![None; black_box(2)];
    g[1] = Some(&SEVEN);
    print_u32(g[1].map_or(0, |v| *v)); print_bool(g[0].is_none());
    let h: Vec<Option<char>> = vec![None; black_box(2)];
    print_usize(h.iter().filter(|o| o.is_none()).count());
    let i: Vec<Option<SL<NonZeroU8>>> = vec![None; black_box(2)];
    print_usize(i.iter().filter(|o| o.is_none()).count());
    // unsafe, every value in its normal representation: all-zero bytes of a wrapping niche
    let z: OArr = unsafe { core::mem::zeroed() };
    print_u32(match black_box(z) { OArr::P(a) => a[0] as u32, OArr::Q => 1, OArr::R => 2 });
    let z: OArrS = unsafe { MaybeUninit::zeroed().assume_init() };
    print_u32(match black_box(z) { OArrS::P(_) => 99, OArrS::Q => 1, OArrS::R => 2 });
    let z: OTupS = unsafe { MaybeUninit::zeroed().assume_init() };
    print_u32(match black_box(z) { OTupS::P(_) => 99, OTupS::Q => 1, OTupS::R => 2 });
    let zv: Vec<OArr> = unsafe { let mut v: Vec<MaybeUninit<OArr>> = Vec::with_capacity(2); v.push(MaybeUninit::zeroed()); v.push(MaybeUninit::zeroed()); core::mem::transmute(v) };
    print_u32(match zv[black_box(1)] { OArr::P(a) => a[0] as u32, OArr::Q => 1, OArr::R => 2 });
    let zb: Box<[OTupS]> = unsafe { Box::<[OTupS]>::new_zeroed_slice(black_box(2)).assume_init() };
    print_u32(match zb[1] { OTupS::P(_) => 99, OTupS::Q => 1, OTupS::R => 2 });
    let zb2: Box<[Option<SL<char>>]> = unsafe { Box::<[Option<SL<char>>]>::new_zeroed_slice(black_box(2)).assume_init() };
    print_u32(match zb2[1] { Some(SL::Live(c)) => 100 + c as u32, Some(_) => 99, None => 1 });
}
