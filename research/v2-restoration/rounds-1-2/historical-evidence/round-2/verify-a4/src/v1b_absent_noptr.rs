//! Verifier probe v1: niche enums with uninhabited variants of several shapes, as register-sized
//! constants (inline match, argument, black_box) and in constant memory (a static table).
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
use core::marker::PhantomData;
use core::num::NonZeroU8;

trait HasOut {
    type Out;
}
impl<T> HasOut for fn() -> T {
    type Out = T;
}
type Never = <fn() -> ! as HasOut>::Out;

#[derive(Clone, Copy)]
enum Void {}
#[derive(Clone, Copy)]
struct Wrap<T>(T);
#[derive(Clone, Copy)]
#[repr(align(4))]
struct Aligned(Void);
#[derive(Clone, Copy)]
enum AllDead {
    A(Void),
    B(Never),
}

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

#[derive(Clone, Copy)]
enum E1 { Live(char), Dead(Never), After }
suite!(s1, E1, [E1::Live('a'), E1::After], |e| match e { E1::Live(c) => c as u32, E1::After => 7, E1::Dead(_) => 99 });

#[derive(Clone, Copy)]
enum E2 { Dead(Void), Live(char), After, Last }
suite!(s2, E2, [E2::Live('b'), E2::After, E2::Last], |e| match e { E2::Live(c) => c as u32, E2::After => 7, E2::Last => 8, E2::Dead(_) => 99 });

#[derive(Clone, Copy)]
enum E3 { Live(char), Dead([u32; 0], Void), After }
suite!(s3, E3, [E3::Live('c'), E3::After], |e| match e { E3::Live(c) => c as u32, E3::After => 7, E3::Dead(..) => 99 });

#[derive(Clone, Copy)]
enum E4 { Live(char), Dead(Void, [u64; 0]), After, Dead2(Infallible), Last }
suite!(s4, E4, [E4::Live('d'), E4::After, E4::Last], |e| match e { E4::Live(c) => c as u32, E4::After => 7, E4::Last => 8, E4::Dead(..) => 99, E4::Dead2(_) => 98 });

#[derive(Clone, Copy)]
enum E5 { Live(char), Dead(Aligned), After }
suite!(s5, E5, [E5::Live('e'), E5::After], |e| match e { E5::Live(c) => c as u32, E5::After => 7, E5::Dead(_) => 99 });

#[derive(Clone, Copy)]
enum E6 { Live(char), Dead(Wrap<Void>), After }
suite!(s6, E6, [E6::Live('f'), E6::After], |e| match e { E6::Live(c) => c as u32, E6::After => 7, E6::Dead(_) => 99 });

#[derive(Clone, Copy)]
enum E7 { Live(char), Dead((Void,)), Mid, Dead2([Void; 2]), After }
suite!(s7, E7, [E7::Live('g'), E7::Mid, E7::After], |e| match e { E7::Live(c) => c as u32, E7::Mid => 6, E7::After => 7, E7::Dead(_) => 99, E7::Dead2(_) => 98 });

#[derive(Clone, Copy)]
enum E8 { Live(char), Empty([Void; 0]), After }
suite!(s8, E8, [E8::Live('h'), E8::Empty([]), E8::After], |e| match e { E8::Live(c) => c as u32, E8::After => 7, E8::Empty(_) => 5 });

#[derive(Clone, Copy)]
enum E9 { Live(char), Ph(PhantomData<Void>), After }
suite!(s9, E9, [E9::Live('i'), E9::Ph(PhantomData), E9::After], |e| match e { E9::Live(c) => c as u32, E9::After => 7, E9::Ph(_) => 5 });

#[derive(Clone, Copy)]
enum E10 { Live(char), Dead(AllDead), After }
suite!(s10, E10, [E10::Live('j'), E10::After], |e| match e { E10::Live(c) => c as u32, E10::After => 7, E10::Dead(_) => 99 });

#[derive(Clone, Copy)]
enum E11 { A, B, Live(char), C, D }
suite!(s11, E11, [E11::A, E11::B, E11::Live('k'), E11::C, E11::D], |e| match e { E11::A => 1, E11::B => 2, E11::Live(c) => c as u32, E11::C => 3, E11::D => 4 });

#[derive(Clone, Copy)]
enum E12 { Dead(Void), A, Live(bool), Dead2(Never), B, Dead3(Void), C }
suite!(s12, E12, [E12::A, E12::Live(true), E12::Live(false), E12::B, E12::C], |e| match e { E12::A => 1, E12::Live(b) => 10 + b as u32, E12::B => 2, E12::C => 3, E12::Dead(_) => 99, E12::Dead2(_) => 98, E12::Dead3(_) => 97 });

#[derive(Clone, Copy)]
enum E13 { Live(char), A, B, Dead(Void) }
suite!(s13, E13, [E13::Live('m'), E13::A, E13::B], |e| match e { E13::Live(c) => c as u32, E13::A => 1, E13::B => 2, E13::Dead(_) => 99 });

#[derive(Clone, Copy)]
enum E14 { Dead(u8, Void), Live(char), After }
suite!(s14, E14, [E14::Live('n'), E14::After], |e| match e { E14::Live(c) => c as u32, E14::After => 7, E14::Dead(..) => 99 });

#[derive(Clone, Copy)]
enum G<T> { Live(char), Mid(T), After }
suite!(s16a, G<Void>, [G::Live('o'), G::After], |e| match e { G::Live(c) => c as u32, G::After => 7, G::Mid(_) => 99 });
suite!(s16b, G<()>, [G::Live('p'), G::Mid(()), G::After], |e| match e { G::Live(c) => c as u32, G::After => 7, G::Mid(_) => 5 });
suite!(s16c, G<[u32; 0]>, [G::Live('q'), G::Mid([]), G::After], |e| match e { G::Live(c) => c as u32, G::After => 7, G::Mid(_) => 5 });
suite!(s16d, G<Never>, [G::Live('r'), G::After], |e| match e { G::Live(c) => c as u32, G::After => 7, G::Mid(_) => 99 });
suite!(s16e, G<([u16; 0], Void)>, [G::Live('s'), G::After], |e| match e { G::Live(c) => c as u32, G::After => 7, G::Mid(_) => 99 });
suite!(s16f, G<(u32, Void)>, [G::Live('t'), G::After], |e| match e { G::Live(c) => c as u32, G::After => 7, G::Mid(_) => 99 });
suite!(s16g, G<Wrap<Wrap<(Void, ())>>>, [G::Live('u'), G::After], |e| match e { G::Live(c) => c as u32, G::After => 7, G::Mid(_) => 99 });

#[derive(Clone, Copy)]
enum E17 { Live(bool), Dead(Void), A, B, C }
suite!(s17, E17, [E17::Live(false), E17::Live(true), E17::A, E17::B, E17::C], |e| match e { E17::Live(b) => 10 + b as u32, E17::A => 1, E17::B => 2, E17::C => 3, E17::Dead(_) => 99 });

#[derive(Clone, Copy)]
enum E18 { Live(NonZeroU8), Dead(Never), A }
suite!(s18, E18, [E18::Live(NonZeroU8::new(200).unwrap()), E18::A], |e| match e { E18::Live(n) => n.get() as u32, E18::A => 1000, E18::Dead(_) => 99 });

#[derive(Clone, Copy)]
enum E18b { Live(NonZeroU8), Dead([u8; 0], Void), A }
suite!(s18b, E18b, [E18b::Live(NonZeroU8::new(201).unwrap()), E18b::A], |e| match e { E18b::Live(n) => n.get() as u32, E18b::A => 1000, E18b::Dead(..) => 99 });

suite!(s19a, Option<E1>, [Some(E1::Live('v')), Some(E1::After), None], |e| match e { Some(E1::Live(c)) => c as u32, Some(E1::After) => 7, Some(E1::Dead(_)) => 99, None => 0 });
suite!(s19b, Option<Option<E3>>, [Some(Some(E3::Live('w'))), Some(Some(E3::After)), Some(None), None], |e| match e { Some(Some(E3::Live(c))) => c as u32, Some(Some(E3::After)) => 7, Some(Some(E3::Dead(..))) => 99, Some(None) => 1, None => 0 });
suite!(s19c, Option<E12>, [Some(E12::A), Some(E12::Live(true)), Some(E12::B), Some(E12::C), None], |e| match e { Some(E12::A) => 1, Some(E12::Live(b)) => 10 + b as u32, Some(E12::B) => 2, Some(E12::C) => 3, Some(_) => 99, None => 0 });

suite!(s20a, Result<bool, Void>, [Ok(true), Ok(false)], |e| match e { Ok(b) => 10 + b as u32, Err(_) => 99 });
suite!(s20b, Option<Result<bool, Void>>, [Some(Ok(true)), Some(Ok(false)), None], |e| match e { Some(Ok(b)) => 10 + b as u32, Some(Err(_)) => 99, None => 0 });
suite!(s20c, Option<Result<Void, bool>>, [Some(Err(true)), Some(Err(false)), None], |e| match e { Some(Err(b)) => 10 + b as u32, Some(Ok(_)) => 99, None => 0 });
suite!(s20d, Option<Result<Never, char>>, [Some(Err('x')), None], |e| match e { Some(Err(c)) => c as u32, Some(Ok(_)) => 99, None => 0 });
suite!(s20e, Option<Result<Infallible, NonZeroU8>>, [Some(Err(NonZeroU8::new(9).unwrap())), None], |e| match e { Some(Err(n)) => n.get() as u32, Some(Ok(_)) => 99, None => 0 });
suite!(s20f, Result<Option<bool>, Infallible>, [Ok(Some(true)), Ok(None)], |e| match e { Ok(Some(b)) => 10 + b as u32, Ok(None) => 0, Err(_) => 99 });

#[derive(Clone, Copy)]
#[repr(u8)]
enum Small { X = 2, Y = 0xFE }
#[derive(Clone, Copy)]
enum E21 { P(Small), Q, Dead(Void), R }
suite!(s21, E21, [E21::P(Small::X), E21::P(Small::Y), E21::Q, E21::R], |e| match e { E21::P(s) => s as u32, E21::Q => 1000, E21::R => 1001, E21::Dead(_) => 99 });
#[derive(Clone, Copy)]
enum E21b { P(Small), Q, R, S }
suite!(s21b, E21b, [E21b::P(Small::X), E21b::P(Small::Y), E21b::Q, E21b::R, E21b::S], |e| match e { E21b::P(s) => s as u32, E21b::Q => 1000, E21b::R => 1001, E21b::S => 1002 });

#[unsafe(no_mangle)]
pub fn rust_entry() {
    s1(); s2(); s3(); s4(); s5(); s6(); s7(); s8(); s9(); s10(); s11(); s12(); s13(); s14();
    s16a(); s16b(); s16c(); s16d(); s16e(); s16f(); s16g();
    s17(); s18(); s18b(); s19a(); s19b(); s19c();
    s20a(); s20b(); s20c(); s20d(); s20e(); s20f();
    s21(); s21b();
}
