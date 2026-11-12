//! Provider crate: closure fn-pointer constants and one-element static slices defined here, used
//! from another crate.
#![no_std]

pub type F = fn(u32) -> u32;
pub fn named(x: u32) -> u32 { x * 3 }
pub const C1: F = |x| x + 7;
pub const C0: fn() -> u32 = || 5;
pub static S1: F = |x| x + 9;
pub const OC: Option<F> = Some(|x| x + 31);
pub struct Op { pub run: fn(u32, u32) -> u32, pub name: &'static str }
pub static OPS: [Op; 2] = [Op { run: |a, b| a + b, name: "add" }, Op { run: |a, b| a * b, name: "mul" }];
pub const TABLE: [F; 3] = [|x| x * 2, named, |x| x + 100];
pub const W64: fn(u64, ()) -> u64 = |a, _| a + (1 << 40);
pub struct G<T>(pub T);
impl<T: Copy + Into<u64>> G<T> {
    pub const CONV: fn(T) -> u64 = |t| t.into() + 400;
}
pub trait HasF { const F: F; }
pub struct I1;
impl HasF for I1 { const F: F = |x| x + 300; }
#[inline(never)]
pub fn get() -> F { C1 }
#[inline]
pub fn get_inline() -> F { C1 }
pub fn call_here(x: u32) -> u32 { C1(x) + S1(x) + TABLE[2](x) }

pub struct N { pub id: u32 }
pub static B: N = N { id: 2 };
pub static ONE: &[N] = core::slice::from_ref(&B);
pub fn one_here() -> &'static [N] { ONE }
