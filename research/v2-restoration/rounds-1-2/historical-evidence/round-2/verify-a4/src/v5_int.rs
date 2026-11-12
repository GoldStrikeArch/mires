//! Verifier probe v5: integer niches as register-sized constants and in constant memory.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cmp::Ordering;
use core::hint::black_box;
use core::num::{NonZeroI8, NonZeroI32, NonZeroI64, NonZeroU128, NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU8, NonZeroUsize};
use core::ops::ControlFlow;
use core::task::Poll;

macro_rules! suite {
    ($f:ident, $T:ty, [$($k:expr),* $(,)?], |$e:ident| $body:expr) => {
        #[inline(never)]
        fn $f() {
            #[inline(never)]
            fn cls($e: $T) -> u32 { $body }
            static TABLE: &[$T] = &[$($k),*];
            $(
                {
                    const K: $T = $k;
                    { let $e = K; print_u32($body); }
                    print_u32(cls(K));
                    print_u32(cls(black_box(K)));
                }
            )*
            let mut i = 0;
            while i < TABLE.len() {
                print_u32(cls(TABLE[black_box(i)]));
                i += 1;
            }
        }
    };
}

#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] #[repr(transparent)] struct Tr<T>(T);
#[derive(Clone, Copy)] struct Named { z: (), b: bool }
#[derive(Clone, Copy)] struct Named2 { b: char, z: [u8; 0] }

suite!(w1, Option<Wrap<bool>>, [Some(Wrap(true)), Some(Wrap(false)), None], |e| match e { Some(Wrap(b)) => 10 + b as u32, None => 0 });
suite!(w2, Option<(bool,)>, [Some((true,)), Some((false,)), None], |e| match e { Some((b,)) => 10 + b as u32, None => 0 });
suite!(w3, Option<((), bool)>, [Some(((), true)), Some(((), false)), None], |e| match e { Some((_, b)) => 10 + b as u32, None => 0 });
suite!(w4, Option<(bool, ())>, [Some((true, ())), None], |e| match e { Some((b, _)) => 10 + b as u32, None => 0 });
suite!(w5, Option<Named>, [Some(Named { z: (), b: true }), Some(Named { z: (), b: false }), None], |e| match e { Some(n) => 10 + n.b as u32, None => 0 });
suite!(w6, Option<Named2>, [Some(Named2 { b: 'q', z: [] }), None], |e| match e { Some(n) => n.b as u32, None => 0 });
suite!(w7, Option<Wrap<Wrap<char>>>, [Some(Wrap(Wrap('r'))), None], |e| match e { Some(Wrap(Wrap(c))) => c as u32, None => 0 });
suite!(w8, Option<Tr<NonZeroU8>>, [Some(Tr(NonZeroU8::new(3).unwrap())), None], |e| match e { Some(Tr(n)) => n.get() as u32, None => 0 });
suite!(w9, Option<Wrap<((), Tr<(bool,)>, [u16; 0])>>, [Some(Wrap(((), Tr((true,)), []))), Some(Wrap(((), Tr((false,)), []))), None], |e| match e { Some(Wrap((_, Tr((b,)), _))) => 10 + b as u32, None => 0 });
suite!(w10, Option<[bool; 1]>, [Some([true]), Some([false]), None], |e| match e { Some([b]) => 10 + b as u32, None => 0 });
suite!(w11, Wrap<Option<bool>>, [Wrap(Some(true)), Wrap(None)], |e| match e { Wrap(Some(b)) => 10 + b as u32, Wrap(None) => 0 });
suite!(w12, (Option<char>,), [(Some('s'),), (None,)], |e| match e { (Some(c),) => c as u32, (None,) => 0 });

suite!(n1, Option<NonZeroU8>, [NonZeroU8::new(255), NonZeroU8::new(1), None], |e| e.map_or(0, |n| n.get() as u32));
suite!(n2, Option<NonZeroU16>, [NonZeroU16::new(65535), None], |e| e.map_or(0, |n| n.get() as u32));
suite!(n3, Option<NonZeroU32>, [NonZeroU32::new(u32::MAX), NonZeroU32::new(77), None], |e| e.map_or(0, |n| n.get()));
suite!(n4, Option<NonZeroU64>, [NonZeroU64::new(u64::MAX), NonZeroU64::new(1 << 40), None], |e| e.map_or(0, |n| (n.get() >> 33) as u32 + (n.get() & 0xff) as u32));
suite!(n5, Option<NonZeroI8>, [NonZeroI8::new(-1), NonZeroI8::new(-128), NonZeroI8::new(127), None], |e| e.map_or(0, |n| (n.get() as i32 + 1000) as u32));
suite!(n6, Option<NonZeroI32>, [NonZeroI32::new(-1), NonZeroI32::new(i32::MIN), None], |e| e.map_or(0, |n| n.get() as u32));
suite!(n7, Option<NonZeroI64>, [NonZeroI64::new(-1), NonZeroI64::new(i64::MIN), NonZeroI64::new(5), None], |e| e.map_or(0, |n| (n.get() >> 60) as u32 ^ n.get() as u32));
suite!(n8, Option<NonZeroUsize>, [NonZeroUsize::new(usize::MAX), NonZeroUsize::new(4), None], |e| e.map_or(0, |n| n.get() as u32));
suite!(n9, Option<NonZeroU128>, [NonZeroU128::new(u128::MAX), NonZeroU128::new(1 << 100), NonZeroU128::new(9), None], |e| e.map_or(0, |n| (n.get() >> 99) as u32 ^ n.get() as u32));
suite!(n10, Option<Option<NonZeroU8>>, [Some(NonZeroU8::new(9)), Some(None), None], |e| match e { Some(Some(n)) => n.get() as u32, Some(None) => 1000, None => 0 });

suite!(c1, Option<char>, [Some('a'), Some('\u{10FFFF}'), Some('\0'), None], |e| e.map_or(7, |c| c as u32));
suite!(c2, Option<Option<char>>, [Some(Some('b')), Some(None), None], |e| match e { Some(Some(c)) => c as u32, Some(None) => 1, None => 0 });
suite!(c3, Option<Option<Option<Option<char>>>>, [Some(Some(Some(Some('c')))), Some(Some(Some(None))), Some(Some(None)), Some(None), None], |e| match e { Some(Some(Some(Some(c)))) => c as u32, Some(Some(Some(None))) => 3, Some(Some(None)) => 2, Some(None) => 1, None => 0 });
suite!(c4, Result<char, ()>, [Ok('d'), Err(())], |e| match e { Ok(c) => c as u32, Err(()) => 0 });
suite!(c5, Result<(), char>, [Ok(()), Err('e')], |e| match e { Ok(()) => 0, Err(c) => c as u32 });
#[derive(Clone, Copy)] enum Tok { Eof, Nl, Ch(char), Tab, Sp, Semi }
suite!(c6, Tok, [Tok::Eof, Tok::Nl, Tok::Ch('f'), Tok::Ch('\u{10FFFF}'), Tok::Tab, Tok::Sp, Tok::Semi], |e| match e { Tok::Eof => 1, Tok::Nl => 2, Tok::Ch(c) => c as u32, Tok::Tab => 3, Tok::Sp => 4, Tok::Semi => 5 });
suite!(c7, Option<Tok>, [Some(Tok::Eof), Some(Tok::Ch('g')), Some(Tok::Semi), None], |e| match e { Some(Tok::Eof) => 1, Some(Tok::Nl) => 2, Some(Tok::Ch(c)) => c as u32, Some(Tok::Tab) => 3, Some(Tok::Sp) => 4, Some(Tok::Semi) => 5, None => 0 });

suite!(b1, Option<bool>, [Some(true), Some(false), None], |e| match e { Some(b) => 10 + b as u32, None => 0 });
suite!(b2, Option<Option<bool>>, [Some(Some(true)), Some(Some(false)), Some(None), None], |e| match e { Some(Some(b)) => 10 + b as u32, Some(None) => 1, None => 0 });
suite!(b3, Result<bool, ()>, [Ok(true), Ok(false), Err(())], |e| match e { Ok(b) => 10 + b as u32, Err(()) => 0 });
#[derive(Clone, Copy)] enum Inner { A, V(bool), B }
#[derive(Clone, Copy)] enum Outer { X, In(Inner), Y }
suite!(b4, Outer, [Outer::X, Outer::In(Inner::A), Outer::In(Inner::V(true)), Outer::In(Inner::V(false)), Outer::In(Inner::B), Outer::Y], |e| match e { Outer::X => 1, Outer::In(Inner::A) => 2, Outer::In(Inner::V(b)) => 10 + b as u32, Outer::In(Inner::B) => 3, Outer::Y => 4 });
suite!(b5, Option<Outer>, [Some(Outer::X), Some(Outer::In(Inner::V(true))), Some(Outer::In(Inner::B)), Some(Outer::Y), None], |e| match e { Some(Outer::X) => 1, Some(Outer::In(Inner::A)) => 2, Some(Outer::In(Inner::V(b))) => 10 + b as u32, Some(Outer::In(Inner::B)) => 3, Some(Outer::Y) => 4, None => 0 });
suite!(b6, Poll<bool>, [Poll::Ready(true), Poll::Ready(false), Poll::Pending], |e| match e { Poll::Ready(b) => 10 + b as u32, Poll::Pending => 0 });
suite!(b7, Poll<Option<bool>>, [Poll::Ready(Some(true)), Poll::Ready(None), Poll::Pending], |e| match e { Poll::Ready(Some(b)) => 10 + b as u32, Poll::Ready(None) => 1, Poll::Pending => 0 });
suite!(b8, ControlFlow<bool, ()>, [ControlFlow::Break(true), ControlFlow::Break(false), ControlFlow::Continue(())], |e| match e { ControlFlow::Break(b) => 10 + b as u32, ControlFlow::Continue(()) => 0 });
suite!(b9, ControlFlow<(), char>, [ControlFlow::Break(()), ControlFlow::Continue('h')], |e| match e { ControlFlow::Break(()) => 0, ControlFlow::Continue(c) => c as u32 });

#[derive(Clone, Copy, PartialEq)] enum Fl { A, B, C }
#[derive(Clone, Copy)] #[repr(i8)] enum Neg { A = -128, B = -3, C = 5 }
#[derive(Clone, Copy)] #[repr(u8)] enum High { A = 250, B = 255 }
#[derive(Clone, Copy)] #[repr(i16)] enum Mid16 { A = -2, B = 300 }
#[derive(Clone, Copy)] #[repr(u32)] enum Wide32 { A = 1, B = 0xFFFF_FFF0 }
#[derive(Clone, Copy)] #[repr(i64)] enum Big { A = -1, B = i64::MAX, C = 7 }
#[derive(Clone, Copy)] #[repr(u64)] enum Big2 { A = 3, B = 1 << 40 }
#[derive(Clone, Copy)] enum One { Only }
suite!(f1, Option<Fl>, [Some(Fl::A), Some(Fl::B), Some(Fl::C), None], |e| match e { Some(f) => 1 + f as u32, None => 0 });
suite!(f2, Option<Option<Fl>>, [Some(Some(Fl::C)), Some(None), None], |e| match e { Some(Some(f)) => 10 + f as u32, Some(None) => 1, None => 0 });
suite!(f3, Option<Ordering>, [Some(Ordering::Less), Some(Ordering::Equal), Some(Ordering::Greater), None], |e| match e { Some(o) => (o as i32 + 5) as u32, None => 0 });
suite!(f4, Option<Option<Ordering>>, [Some(Some(Ordering::Less)), Some(Some(Ordering::Greater)), Some(None), None], |e| match e { Some(Some(o)) => (o as i32 + 5) as u32, Some(None) => 1, None => 0 });
suite!(f5, Option<Neg>, [Some(Neg::A), Some(Neg::B), Some(Neg::C), None], |e| match e { Some(n) => (n as i32 + 500) as u32, None => 0 });
suite!(f6, Option<Option<Neg>>, [Some(Some(Neg::A)), Some(Some(Neg::C)), Some(None), None], |e| match e { Some(Some(n)) => (n as i32 + 500) as u32, Some(None) => 1, None => 0 });
suite!(f7, Option<High>, [Some(High::A), Some(High::B), None], |e| match e { Some(h) => h as u32, None => 0 });
suite!(f8, Option<Option<High>>, [Some(Some(High::A)), Some(Some(High::B)), Some(None), None], |e| match e { Some(Some(h)) => h as u32, Some(None) => 1, None => 0 });
suite!(f9, Option<Mid16>, [Some(Mid16::A), Some(Mid16::B), None], |e| match e { Some(m) => (m as i32 + 5000) as u32, None => 0 });
suite!(f10, Option<Wide32>, [Some(Wide32::A), Some(Wide32::B), None], |e| match e { Some(m) => m as u32, None => 0 });
suite!(f11, Option<Big>, [Some(Big::A), Some(Big::B), Some(Big::C), None], |e| match e { Some(m) => ((m as i64) >> 40) as u32 ^ (m as i64) as u32, None => 0 });
suite!(f12, Option<Big2>, [Some(Big2::A), Some(Big2::B), None], |e| match e { Some(m) => ((m as u64) >> 38) as u32 + (m as u64) as u32, None => 0 });
suite!(f13, Option<One>, [Some(One::Only), None], |e| match e { Some(One::Only) => 1, None => 0 });
suite!(f14, Option<()>, [Some(()), None], |e| match e { Some(()) => 1, None => 0 });
suite!(f15, Result<Fl, Neg>, [Ok(Fl::B), Err(Neg::A), Err(Neg::C)], |e| match e { Ok(f) => f as u32, Err(n) => (n as i32 + 500) as u32 });
#[derive(Clone, Copy)] enum Many { V(Neg), A, B, C, D }
suite!(f16, Many, [Many::V(Neg::A), Many::V(Neg::B), Many::V(Neg::C), Many::A, Many::B, Many::C, Many::D], |e| match e { Many::V(n) => (n as i32 + 500) as u32, Many::A => 1, Many::B => 2, Many::C => 3, Many::D => 4 });
#[derive(Clone, Copy)] enum ManyH { A, B, V(High), C }
suite!(f17, ManyH, [ManyH::A, ManyH::B, ManyH::V(High::A), ManyH::V(High::B), ManyH::C], |e| match e { ManyH::A => 1, ManyH::B => 2, ManyH::V(h) => h as u32, ManyH::C => 3 });
suite!(f18, Option<ManyH>, [Some(ManyH::A), Some(ManyH::V(High::B)), Some(ManyH::C), None], |e| match e { Some(ManyH::A) => 1, Some(ManyH::B) => 2, Some(ManyH::V(h)) => h as u32, Some(ManyH::C) => 3, None => 0 });
#[derive(Clone, Copy)] enum ManyZ { A, V(NonZeroU8) }
suite!(f19, ManyZ, [ManyZ::A, ManyZ::V(NonZeroU8::new(1).unwrap()), ManyZ::V(NonZeroU8::new(255).unwrap())], |e| match e { ManyZ::A => 0, ManyZ::V(n) => n.get() as u32 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    w1(); w2(); w3(); w4(); w5(); w6(); w7(); w8(); w9(); w10(); w11(); w12();
    n1(); n2(); n3(); n4(); n5(); n6(); n7(); n8(); n9(); n10();
    c1(); c2(); c3(); c4(); c5(); c6(); c7();
    b1(); b2(); b3(); b4(); b5(); b6(); b7(); b8(); b9();
    f1(); f2(); f3(); f4(); f5(); f6(); f7(); f8(); f9(); f10(); f11(); f12(); f13(); f14(); f15(); f16(); f17(); f18(); f19();
}
