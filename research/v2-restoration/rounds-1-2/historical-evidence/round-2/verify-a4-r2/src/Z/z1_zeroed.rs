#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code, invalid_value)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
use core::mem::MaybeUninit;
use core::num::NonZeroU8;

#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi2 { A = 200, B = 254 }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi { A = 200, B = 253 }
#[derive(Clone, Copy)] #[repr(u16)] enum W16 { A = 0xFFF0, B = 0xFFFE }
#[derive(Clone, Copy)] #[repr(u64)] enum W64 { A = 0xFFFF_FFFF_FFFF_FF00, B = 0xFFFF_FFFF_FFFF_FFFE }
// niche values 255, 0: zero is R
#[derive(Clone, Copy)] enum O2 { P(Hi2), Q, R }
// niche values 254, 255, 0: zero is S
#[derive(Clone, Copy)] enum O3 { P(Hi), Q, R, S }
// 255 -> D (kept, not absent), 0 -> Q
#[derive(Clone, Copy)] enum OD { P(Hi2), D([u32; 0], Void), Q }
// absent dead variants do not take a value: 255 -> Q, 0 -> R
#[derive(Clone, Copy)] enum OA { A(Void), P(Hi2), B(Infallible), Q, C((Void,)), R }
#[derive(Clone, Copy)] enum O16 { P(W16), Q, R }
#[derive(Clone, Copy)] enum O64 { P(W64), Q, R }
#[derive(Clone, Copy)] enum OW { P(Wrap<((), Hi2)>), Q, R }
#[derive(Clone, Copy)] enum OS { P(SL<Hi2>), Q, R }
#[derive(Clone, Copy)] enum OArr { P([Hi2; 2]), Q, R }
#[derive(Clone, Copy)] enum OTup { P(u32, Hi2), Q, R }
#[derive(Clone, Copy)] enum OTup2 { P(Hi2, u32), Q, R }
#[derive(Clone, Copy)] enum OBig { P(u64, SL<Wrap<Hi2>>, u16), Q, R }

macro_rules! z {
    ($T:ty, |$e:ident| $body:expr) => {{
        #[inline(never)]
        fn cls($e: $T) -> u32 { $body }
        let a: $T = unsafe { core::mem::zeroed() };
        print_u32(cls(a));
        let b: $T = unsafe { MaybeUninit::zeroed().assume_init() };
        print_u32(cls(black_box(b)));
        let mut c = MaybeUninit::<$T>::uninit();
        unsafe { core::ptr::write_bytes(c.as_mut_ptr(), 0, 1) };
        print_u32(cls(unsafe { c.assume_init() }));
        let d: [$T; 2] = unsafe { core::mem::zeroed() };
        print_u32(cls(d[black_box(1)]));
    }};
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    z!(Option<&'static u32>, |e| match e { Some(r) => *r, None => 1 });
    z!(Option<NonZeroU8>, |e| match e { Some(n) => n.get() as u32, None => 1 });
    z!(Option<char>, |e| match e { Some(c) => 100 + c as u32, None => 1 });
    z!(Option<bool>, |e| match e { Some(b) => 100 + b as u32, None => 1 });
    z!(Option<Option<bool>>, |e| match e { Some(Some(b)) => 100 + b as u32, Some(None) => 2, None => 1 });
    z!(Option<[bool; 2]>, |e| match e { Some(b) => 100 + b[0] as u32 + b[1] as u32, None => 1 });
    z!(Option<(u64, bool)>, |e| match e { Some((n, b)) => 100 + n as u32 + b as u32, None => 1 });
    z!(Option<SL<char>>, |e| match e { Some(SL::Live(c)) => 100 + c as u32, Some(SL::Never(..)) => 99, None => 1 });
    z!(Option<[SL<bool>; 2]>, |e| match e { Some([SL::Live(a), SL::Live(b)]) => 100 + a as u32 + b as u32, Some(_) => 99, None => 1 });
    z!(O2, |e| match e { O2::P(h) => h as u32, O2::Q => 1, O2::R => 2 });
    z!(O3, |e| match e { O3::P(h) => h as u32, O3::Q => 1, O3::R => 2, O3::S => 3 });
    z!(O16, |e| match e { O16::P(h) => h as u32, O16::Q => 1, O16::R => 2 });
    z!(O64, |e| match e { O64::P(h) => (h as u64 % 1000) as u32, O64::Q => 1, O64::R => 2 });
    z!(OW, |e| match e { OW::P(w) => (w.0).1 as u32, OW::Q => 1, OW::R => 2 });
    z!(OS, |e| match e { OS::P(SL::Live(h)) => h as u32, OS::P(_) => 99, OS::Q => 1, OS::R => 2 });
    z!(Option<Option<O2>>, |e| match e { Some(Some(O2::P(h))) => h as u32, Some(Some(O2::Q)) => 1, Some(Some(O2::R)) => 2, Some(None) => 3, None => 4 });
}
