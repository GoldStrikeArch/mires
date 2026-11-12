//! Verifier probe v2: a niche enum whose untagged payload is an enum rustc lays out as a single
//! variant that is not variant 0 (`Result<!, char>`): the niche sits in the second variant's field.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;

trait HasOut {
    type Out;
}
impl<T> HasOut for fn() -> T {
    type Out = T;
}
type Never = <fn() -> ! as HasOut>::Out;

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

suite!(a, Option<Result<Never, char>>, [Some(Err('x')), None], |e| match e { Some(Err(c)) => c as u32, Some(Ok(_)) => 99, None => 0 });
suite!(b, Option<Result<Infallible, char>>, [Some(Err('y')), None], |e| match e { Some(Err(c)) => c as u32, Some(Ok(_)) => 99, None => 0 });
suite!(c, Option<Result<(Infallible, ()), char>>, [Some(Err('z')), None], |e| match e { Some(Err(c)) => c as u32, Some(Ok(_)) => 99, None => 0 });

#[derive(Clone, Copy)]
enum Second { A((), Infallible), B(char) }
suite!(d, Option<Second>, [Some(Second::B('w')), None], |e| match e { Some(Second::B(c)) => c as u32, Some(Second::A(..)) => 99, None => 0 });

#[derive(Clone, Copy)]
enum Third { A(Infallible), B((), Infallible), C(bool) }
suite!(e2, Option<Third>, [Some(Third::C(true)), Some(Third::C(false)), None], |e| match e { Some(Third::C(b)) => 10 + b as u32, Some(_) => 99, None => 0 });

#[derive(Clone, Copy)]
#[repr(u8)]
enum Small { X = 2, Y = 0xFE }
#[derive(Clone, Copy)]
enum W { P(Result<Infallible, Small>), Q, R, S }
suite!(f, W, [W::P(Err(Small::X)), W::P(Err(Small::Y)), W::Q, W::R, W::S], |e| match e { W::P(Err(s)) => s as u32, W::P(Ok(_)) => 99, W::Q => 1000, W::R => 1001, W::S => 1002 });
#[derive(Clone, Copy)]
enum W2 { P(Result<Never, Small>), Q, R, S }
suite!(g, W2, [W2::P(Err(Small::X)), W2::P(Err(Small::Y)), W2::Q, W2::R, W2::S], |e| match e { W2::P(Err(s)) => s as u32, W2::P(Ok(_)) => 99, W2::Q => 1000, W2::R => 1001, W2::S => 1002 });

#[inline(never)]
fn first_err() -> u32 {
    // The ordinary spelling: a fallible-less parser result kept in a constant.
    const PARSED: Option<Result<Never, char>> = Some(Err('q'));
    match PARSED {
        Some(Err(c)) => c as u32,
        Some(Ok(never)) => match never {},
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    a(); b(); c(); d(); e2(); f(); g();
    print_u32(first_err());
}
