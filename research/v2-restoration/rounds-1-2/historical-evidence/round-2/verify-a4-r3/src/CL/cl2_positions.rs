//! Closure fn pointers in every constant position.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

fn named(x: u32) -> u32 { x * 3 }
type F = fn(u32) -> u32;

const ARR: [F; 3] = [|x| x + 1, named, |x| x * x];
const SLICE: &[F] = &[|x| x + 10, |x| x + 20, named];
static SSLICE: &[F] = &[|x| x + 30, named, |x| x + 40];
const TUP: (F, u32, fn() -> u32) = (|x| x + 50, 7, || 51);
static STUP: (u8, F) = (1, |x| x + 60);
const OPT: Option<F> = Some(|x| x + 70);
const OPT2: Option<Option<F>> = Some(Some(|x| x + 80));
const OPT2N: Option<Option<F>> = Some(None);
const RES: Result<F, u32> = Ok(|x| x + 90);
const RESE: Result<u32, F> = Err(|x| x + 95);
enum En { A(F), B(Option<fn() -> u32>), C, D { f: fn(u32, u32) -> u32, k: u32 } }
const EA: En = En::A(|x| x + 100);
const EB: En = En::B(Some(|| 110));
const EBN: En = En::B(None);
const EC: En = En::C;
const ED: En = En::D { f: |a, b| a * b + 120, k: 3 };
static SEN: [En; 4] = [En::A(|x| x + 130), En::B(Some(|| 140)), En::C, En::D { f: |a, b| a + b + 150, k: 4 }];
const PROM: &Option<F> = &Some(|x| x + 160);
const PROMF: &F = &((|x| x + 170) as F);
static mut SM: F = |x| x + 180;
static mut SMO: Option<F> = Some(|x| x + 190);
struct VTable { get: fn(&u32) -> u32, set: fn(&mut u32, u32), name: &'static str, drop: Option<fn(&mut u32)> }
static VT: VTable = VTable { get: |r| *r + 1, set: |r, v| *r = v * 2, name: "vt", drop: Some(|r| *r = 0) };
const CVT: &VTable = &VTable { get: |r| *r + 2, set: |r, v| *r = v * 3, name: "cvt", drop: None };
static VTS: [&VTable; 2] = [&VT, CVT];
struct Wrap(F);
struct Wrap2 { a: Wrap, b: (F, F) }
const WR: Wrap2 = Wrap2 { a: Wrap(|x| x + 200), b: (|x| x + 210, named) };
#[repr(transparent)]
struct Tr(F);
const TR: Tr = Tr(|x| x + 220);
const OTR: Option<Tr> = Some(Tr(|x| x + 230));
#[derive(Clone, Copy)]
union Un { f: F, g: fn(u32) -> u32 }
const UN: Un = Un { f: |x| x + 240 };
const BLOCK: F = { let f = |x| x + 250; f };
const BLOCK2: F = { const INNER: F = |x| x + 260; INNER };
const fn mk() -> F { |x| x + 270 }
const VIAFN: F = mk();
static VIAFN_S: F = mk();
const fn pick(i: u32) -> F { if i == 0 { |x| x + 280 } else { named } }
const PICK0: F = pick(0);
const PICK1: F = pick(1);

trait HasF { const F: F; const OF: Option<F>; }
struct I1; struct I2;
impl HasF for I1 { const F: F = |x| x + 300; const OF: Option<F> = Some(|x| x + 310); }
impl HasF for I2 { const F: F = named; const OF: Option<F> = None; }
#[inline(never)]
fn use_assoc<T: HasF>(x: u32) -> u32 { (T::F)(x) + T::OF.map_or(1000, |f| f(x)) }

struct G<T>(T);
impl<T: Copy + Into<u64>> G<T> {
    const CONV: fn(T) -> u64 = |t| t.into() + 400;
    const PAIR: fn(T, T) -> (T, T) = |a, b| (b, a);
    const TABLE: [fn(T) -> u64; 2] = [|t| t.into() * 2, |t| t.into() * 3];
}
#[inline(never)]
fn use_generic<T: Copy + Into<u64>>(a: T, b: T) -> u64 {
    let (x, y) = (G::<T>::PAIR)(a, b);
    (G::<T>::CONV)(x) * 1000 + (G::<T>::TABLE[black_box(1)])(y) + (G::<T>::TABLE[black_box(0)])(a)
}
fn generic_fn<T: Copy + Into<u64>>(t: T) -> u64 {
    let f: fn(T) -> u64 = const { |t: T| -> u64 { core::mem::size_of::<T>() as u64 } };
    f(t) * 100000 + (G::<T>::CONV)(t)
}

#[inline(never)]
fn idx(i: usize) -> usize { black_box(i) }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    for i in 0..3 { print_u32(ARR[idx(i)](5)); print_u32(SLICE[idx(i)](5)); print_u32(SSLICE[idx(i)](5)); }
    print_u32((TUP.0)(1)); print_u32(TUP.1); print_u32((TUP.2)());
    print_u32((STUP.1)(1)); print_u32((black_box(&STUP).1)(2));
    print_u32(OPT.map_or(0, |f| f(1))); print_u32(black_box(OPT).map_or(0, |f| f(2)));
    print_bool(OPT.is_some());
    print_u32(match OPT2 { Some(Some(f)) => f(1), Some(None) => 1, None => 2 });
    print_u32(match black_box(OPT2N) { Some(Some(f)) => f(1), Some(None) => 1, None => 2 });
    print_u32(match RES { Ok(f) => f(1), Err(e) => e }); print_u32(match black_box(RESE) { Ok(v) => v, Err(f) => f(1) });
    for e in [EA, EB, EBN, EC, ED] {
        print_u32(match black_box(e) { En::A(f) => f(1), En::B(Some(f)) => f(), En::B(None) => 2, En::C => 3, En::D { f, k } => f(k, 5) });
    }
    for i in 0..4 {
        print_u32(match &SEN[idx(i)] { En::A(f) => f(1), En::B(Some(f)) => f(), En::B(None) => 2, En::C => 3, En::D { f, k } => f(*k, 5) });
    }
    print_u32(PROM.map_or(0, |f| f(1))); print_u32(black_box(PROM).map_or(0, |f| f(2)));
    print_u32(PROMF(1)); print_u32(black_box(PROMF)(2));
    unsafe {
        let f = SM; print_u32(f(1));
        SM = |x| x + 185; let f = SM; print_u32(f(1));
        SM = named; let f = SM; print_u32(f(1));
        let o = SMO; print_u32(o.map_or(0, |f| f(1)));
        SMO = None; let o = SMO; print_u32(o.map_or(0, |f| f(1)));
        SMO = OPT; let o = SMO; print_u32(o.map_or(0, |f| f(1)));
    }
    for i in 0..2 {
        let vt = VTS[idx(i)];
        let mut v = 5u32;
        print_str(vt.name); print_u32((vt.get)(&v)); (vt.set)(&mut v, 4); print_u32(v);
        if let Some(d) = vt.drop { d(&mut v); } print_u32(v);
    }
    print_u32((WR.a.0)(1)); print_u32((WR.b.0)(1)); print_u32((black_box(WR.b.1))(1));
    print_u32((TR.0)(1)); print_u32(black_box(OTR).map_or(0, |t| (t.0)(1)));
    print_u32(unsafe { (UN.f)(1) }); print_u32(unsafe { (black_box(UN).g)(2) });
    print_u32(BLOCK(1)); print_u32(BLOCK2(1)); print_u32(VIAFN(1)); print_u32(VIAFN_S(1)); print_u32(black_box(VIAFN)(2));
    print_u32(PICK0(1)); print_u32(PICK1(1));
    print_u32(use_assoc::<I1>(1)); print_u32(use_assoc::<I2>(1));
    print_u64(use_generic(1u8, 2u8)); print_u64(use_generic(3u32, 4u32)); print_u64(use_generic(5u64, 1u64 << 40));
    print_u64(generic_fn(9u16));
}
