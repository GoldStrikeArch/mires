#![no_std]
#![no_main]
#![allow(unused, static_mut_refs, dangerous_implicit_autorefs)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
#[inline(never)] fn sum(s: &[u32]) -> u32 { let mut t = 0; for v in s { t = t * 3 + *v; } t }
#[inline(never)] fn show(s: &[u32]) { print_usize(s.len()); print_u32(sum(s)); }
#[inline(never)] fn showo(o: Option<&[u32]>) { match o { Some(s) => show(s), None => print_str("none") } }

#[derive(Clone, Copy, PartialEq)] struct Z;
static ZA: [(); 4] = [(); 4];
static ZS: &[()] = &ZA;
static ZT: &[()] = ZA.split_at(2).1;
static ZE: &[()] = ZA.split_at(4).1;
static ZZ: [Z; 3] = [Z, Z, Z];
static ZZS: &[Z] = &ZZ;
static ZO: Option<&[Z]> = Some(ZZ.split_at(1).1);
static E: [u32; 0] = [];
static ES: &[u32] = &E;
static EO: Option<&[u32]> = Some(&E);
static ONE: [u32; 1] = [9];
static OS: &[u32] = &ONE;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_usize(ZS.len()); print_usize(ZT.len()); print_usize(ZE.len()); print_usize(ZZS.len());
    print_usize(black_box(ZS).iter().count()); print_usize(black_box(ZT).iter().count());
    print_usize(ZO.map_or(99, |s| s.len())); print_usize(black_box(ZO).map_or(99, |s| s.iter().filter(|z| **z == Z).count()));
    print_bool(ZS.get(3).is_some()); print_bool(ZS.get(4).is_some()); print_bool(ZT.get(2).is_some());
    show(ES); showo(EO); showo(black_box(EO)); show(OS);
    print_bool(core::ptr::eq(OS.as_ptr(), &ONE[0])); print_bool(core::ptr::eq(ES.as_ptr(), E.as_ptr()));
    print_bool(core::ptr::eq(ZS.as_ptr(), ZA.as_ptr()));
}
