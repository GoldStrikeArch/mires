//! `Fn::call`, `FnMut::call_mut`, `FnOnce::call_once` named as paths in constants: extern "rust-call"
//! pointers to closure instances, which take the environment explicitly and must stay bare.
#![no_std]
#![no_main]
#![feature(unboxed_closures, fn_traits)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;

struct M1<F>(F);
impl<F: Fn(u32) -> u32> M1<F> {
    const CALL: extern "rust-call" fn(&F, (u32,)) -> u32 = <F as Fn<(u32,)>>::call;
    const MUT: extern "rust-call" fn(&mut F, (u32,)) -> u32 = <F as FnMut<(u32,)>>::call_mut;
    const ONCE: extern "rust-call" fn(F, (u32,)) -> u32 = <F as FnOnce<(u32,)>>::call_once;
    const TABLE: [extern "rust-call" fn(&F, (u32,)) -> u32; 2] = [<F as Fn<(u32,)>>::call, <F as Fn<(u32,)>>::call];
    const OPT: Option<extern "rust-call" fn(&F, (u32,)) -> u32> = Some(<F as Fn<(u32,)>>::call);
}
struct M0<F>(F);
impl<F: Fn() -> u32> M0<F> {
    const CALL: extern "rust-call" fn(&F, ()) -> u32 = <F as Fn<()>>::call;
    const MUT: extern "rust-call" fn(&mut F, ()) -> u32 = <F as FnMut<()>>::call_mut;
    const ONCE: extern "rust-call" fn(F, ()) -> u32 = <F as FnOnce<()>>::call_once;
}
struct M2<F>(F);
impl<F: FnMut(u32, u64) -> u64> M2<F> {
    const MUT: extern "rust-call" fn(&mut F, (u32, u64)) -> u64 = <F as FnMut<(u32, u64)>>::call_mut;
    const ONCE: extern "rust-call" fn(F, (u32, u64)) -> u64 = <F as FnOnce<(u32, u64)>>::call_once;
}
struct MO<F>(F);
impl<F: FnOnce(u32) -> u32> MO<F> {
    const ONCE: extern "rust-call" fn(F, (u32,)) -> u32 = <F as FnOnce<(u32,)>>::call_once;
}

#[inline(never)]
fn m1<F: Fn(u32) -> u32 + Copy>(mut f: F) {
    print_u32(M1::<F>::CALL(&f, (5,)));
    print_u32(black_box(M1::<F>::CALL)(&f, (6,)));
    print_u32(M1::<F>::MUT(&mut f, (7,)));
    print_u32(black_box(M1::<F>::MUT)(&mut f, (8,)));
    print_u32(M1::<F>::ONCE(f, (9,)));
    print_u32(black_box(M1::<F>::ONCE)(f, (10,)));
    print_u32(M1::<F>::TABLE[black_box(1)](&f, (11,)));
    print_u32(M1::<F>::OPT.map_or(0, |g| g(&f, (12,))));
}
#[inline(never)]
fn m0<F: Fn() -> u32 + Copy>(mut f: F) {
    print_u32(M0::<F>::CALL(&f, ()));
    print_u32(black_box(M0::<F>::MUT)(&mut f, ()));
    print_u32(black_box(M0::<F>::ONCE)(f, ()));
}
#[inline(never)]
fn m2<F: FnMut(u32, u64) -> u64>(mut f: F) {
    print_u64(M2::<F>::MUT(&mut f, (1, 1 << 40)));
    print_u64(black_box(M2::<F>::MUT)(&mut f, (2, 3)));
    print_u64(black_box(M2::<F>::ONCE)(f, (4, 5)));
}
#[inline(never)]
fn mo<F: FnOnce(u32) -> u32>(f: F) {
    print_u32(black_box(MO::<F>::ONCE)(f, (3,)));
}
struct NotCopy(u32);

// A named fn item through the same trait paths (FnPtrShim / fn item instances; not closures).
fn named(x: u32) -> u32 { x * 3 }

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // non-capturing
    m1(|x| x + 7);
    m1(|x| x * 3);
    m0(|| 41);
    m2(|a, b| a as u64 + b);
    mo(|x| x + 1);
    // capturing
    let k = black_box(100u32);
    m1(move |x| x + k);
    m0(move || k + 1);
    let mut acc = 0u64;
    m2(|a, b| { acc += a as u64 + b; acc });
    print_u64(acc);
    let nc = NotCopy(black_box(50));
    mo(move |x| { let n = nc; n.0 + x });
    // fn items and fn pointers through the same paths
    m1(named);
    m1(named as fn(u32) -> u32);
    // the same closures coerced as plain fn pointers next to the rust-call forms
    const P: fn(u32) -> u32 = |x| x + 7;
    print_u32(P(1));
    m1(P);
}
