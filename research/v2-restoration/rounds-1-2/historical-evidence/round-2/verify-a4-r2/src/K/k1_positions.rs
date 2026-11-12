#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cmp::Ordering;
use core::convert::Infallible;
use core::hint::black_box;
use core::num::{FpCategory, NonZeroI64, NonZeroU8, NonZeroUsize};
use core::ops::ControlFlow;
use core::task::Poll;

#[derive(Clone, Copy, PartialEq)] enum Void {}
#[derive(Clone, Copy, PartialEq)] struct Wrap<T>(T);
#[derive(Clone, Copy, PartialEq)] enum SL<T> { Never((), Infallible), Live(T) }
#[derive(Clone, Copy, PartialEq)] #[repr(u8)] enum Hi2 { A = 200, B = 254 }
#[derive(Clone, Copy, PartialEq)] enum O2 { P(Hi2), Q, R }
#[derive(Clone, Copy, PartialEq)] enum Dead3 { Live(char), Dead([u32; 0], Void), After }

trait HasK { const K: Option<SL<bool>>; const N: Option<SL<bool>> = None; const W: O2; }
struct A; struct B; struct C;
impl HasK for A { const K: Option<SL<bool>> = Some(SL::Live(true)); const W: O2 = O2::R; }
impl HasK for B { const K: Option<SL<bool>> = None; const N: Option<SL<bool>> = Some(SL::Live(false)); const W: O2 = O2::P(Hi2::B); }
impl HasK for C { const K: Option<SL<bool>> = Some(SL::Live(false)); const W: O2 = O2::Q; }

#[inline(never)] fn sl(o: Option<SL<bool>>) -> u32 { match o { Some(SL::Live(b)) => 10 + b as u32, Some(SL::Never(..)) => 99, None => 0 } }
#[inline(never)] fn o2(o: O2) -> u32 { match o { O2::P(h) => h as u32, O2::Q => 1, O2::R => 2 } }
#[inline(never)] fn generic<T: HasK>() { print_u32(sl(T::K)); print_u32(sl(T::N)); print_u32(o2(T::W)); print_u32(sl(black_box(T::K))); print_u32(o2(black_box(T::W))); print_bool(T::W == O2::R); print_bool(T::K == Some(SL::Live(true))); }

struct Gen<T>(T);
impl<T: Copy> Gen<T> {
    const NONE: Option<SL<T>> = None;
    const DEAD: Dead3 = Dead3::After;
    const SIZED: Option<NonZeroUsize> = NonZeroUsize::new(core::mem::size_of::<T>());
}
#[inline(never)] fn cg<const N: u8>() -> Option<NonZeroU8> { const { NonZeroU8::new(N) } }
#[inline(never)] fn cg2<const B: bool>() -> O2 { const { if B { O2::R } else { O2::P(Hi2::A) } } }
#[inline(never)] fn cg3<const C: char>() -> Option<SL<char>> { const { if C == 'x' { None } else { Some(SL::Live(C)) } } }

const POLL_R: Poll<bool> = Poll::Ready(true);
const POLL_P: Poll<bool> = Poll::Pending;
const OPOLL: Option<Poll<Option<bool>>> = Some(Poll::Ready(None));
const OPOLL2: Option<Poll<Option<bool>>> = Some(Poll::Pending);
const RFMT: Result<bool, core::fmt::Error> = Err(core::fmt::Error);
const RFMT2: Result<bool, core::fmt::Error> = Ok(false);
const OORD: Option<Option<Ordering>> = Some(None);
const OORD2: Option<Option<Ordering>> = Some(Some(Ordering::Less));
const FPC: Option<FpCategory> = Some(1.5f64.classify());
const FPN: Option<FpCategory> = None;
const CF: ControlFlow<bool, ()> = ControlFlow::Continue(());
const CF2: ControlFlow<bool, ()> = ControlFlow::Break(true);
const CHK: Option<char> = char::from_u32(0x41);
const CHK2: Option<char> = char::from_u32(0xD800);
const DIG: Option<char> = char::from_digit(7, 10);
const NZ: Option<NonZeroI64> = NonZeroI64::new(-5);
const NZ0: Option<NonZeroI64> = NonZeroI64::new(0);
const FIRST: Option<&u8> = b"abc".first();
const RI: Result<bool, Infallible> = Ok(true);
const ORI: Option<Result<bool, Infallible>> = Some(Ok(false));
const OI: Option<Infallible> = None;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    generic::<A>(); generic::<B>(); generic::<C>();
    print_u32(match Gen::<u64>::NONE { Some(_) => 1, None => 0 });
    print_u32(match black_box(Gen::<char>::NONE) { Some(_) => 1, None => 0 });
    print_u32(match black_box(Gen::<u8>::DEAD) { Dead3::Live(c) => c as u32, Dead3::After => 7, Dead3::Dead(..) => 99 });
    print_u32(Gen::<u64>::SIZED.map_or(0, |n| n.get() as u32)); print_u32(black_box(Gen::<()>::SIZED).map_or(0, |n| n.get() as u32));
    print_u32(cg::<0>().map_or(0, |n| n.get() as u32)); print_u32(cg::<9>().map_or(0, |n| n.get() as u32));
    print_u32(o2(cg2::<true>())); print_u32(o2(cg2::<false>()));
    print_u32(match cg3::<'x'>() { Some(SL::Live(c)) => c as u32, Some(_) => 99, None => 0 }); print_u32(match cg3::<'y'>() { Some(SL::Live(c)) => c as u32, Some(_) => 99, None => 0 });
    print_u32(match black_box(POLL_R) { Poll::Ready(b) => 10 + b as u32, Poll::Pending => 0 }); print_u32(match black_box(POLL_P) { Poll::Ready(b) => 10 + b as u32, Poll::Pending => 0 });
    print_u32(match black_box(OPOLL) { Some(Poll::Ready(Some(b))) => 10 + b as u32, Some(Poll::Ready(None)) => 3, Some(Poll::Pending) => 2, None => 0 });
    print_u32(match black_box(OPOLL2) { Some(Poll::Ready(Some(b))) => 10 + b as u32, Some(Poll::Ready(None)) => 3, Some(Poll::Pending) => 2, None => 0 });
    print_u32(match OPOLL { Some(Poll::Ready(Some(b))) => 10 + b as u32, Some(Poll::Ready(None)) => 3, Some(Poll::Pending) => 2, None => 0 });
    print_u32(match black_box(RFMT) { Ok(b) => 10 + b as u32, Err(_) => 1 }); print_u32(match black_box(RFMT2) { Ok(b) => 10 + b as u32, Err(_) => 1 });
    print_i32(match black_box(OORD) { Some(Some(o)) => o as i32, Some(None) => 5, None => 6 }); print_i32(match black_box(OORD2) { Some(Some(o)) => o as i32, Some(None) => 5, None => 6 });
    print_bool(black_box(FPC) == Some(FpCategory::Normal)); print_bool(black_box(FPN).is_none());
    print_u32(match black_box(CF) { ControlFlow::Continue(()) => 1, ControlFlow::Break(b) => 10 + b as u32 }); print_u32(match black_box(CF2) { ControlFlow::Continue(()) => 1, ControlFlow::Break(b) => 10 + b as u32 });
    print_u32(black_box(CHK).map_or(0, |c| c as u32)); print_u32(black_box(CHK2).map_or(0, |c| c as u32)); print_u32(black_box(DIG).map_or(0, |c| c as u32));
    print_i64(black_box(NZ).map_or(0, |n| n.get())); print_i64(black_box(NZ0).map_or(0, |n| n.get()));
    print_u32(black_box(FIRST).map_or(0, |b| *b as u32));
    print_u32(match black_box(RI) { Ok(b) => 10 + b as u32, Err(_) => 99 }); print_u32(match black_box(ORI) { Some(Ok(b)) => 10 + b as u32, Some(Err(_)) => 99, None => 0 });
    print_bool(black_box(OI).is_none());
    // comparisons and patterns against constants
    const KP: O2 = O2::R; const KQ: Option<SL<bool>> = Some(SL::Live(true));
    let v = black_box(O2::R); print_bool(v == KP); print_bool(matches!(v, KP)); print_bool(black_box(O2::Q) == KP);
    let w = black_box(Some(SL::Live(true))); print_bool(w == KQ); print_bool(matches!(w, KQ)); print_bool(black_box(None) == KQ);
    print_bool(black_box(Dead3::After) == Dead3::After); print_bool(black_box(Dead3::Live('a')) == Dead3::After);
}
