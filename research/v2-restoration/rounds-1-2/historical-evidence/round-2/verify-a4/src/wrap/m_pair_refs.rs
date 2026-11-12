#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
#[allow(unused_imports)]
use core::{cell::{Cell, UnsafeCell}, mem::{ManuallyDrop, MaybeUninit}, pin::Pin, ptr::NonNull, sync::atomic::{AtomicPtr, AtomicU32, Ordering}};
static TARGET: u32 = 77;
static ATOM: AtomicU32 = AtomicU32::new(5);
fn seven() -> u32 { 7 }
fn add1(x: u32) -> u32 { x + 1 }
#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] #[repr(transparent)] struct Tr<T>(T);
#[derive(Clone, Copy)] #[repr(transparent)] struct TrZ<T>((), T, [u8; 0]);
#[derive(Clone, Copy)] struct Named<T> { z: (), inner: T }
#[derive(Clone, Copy)] union Un { r: &'static u32, n: usize }
#[derive(Clone, Copy)] union UnF { f: fn() -> u32, n: usize }
const C: Option<(&'static u8, &'static u8)> = Some((&1, &2)); const D: Option<(u64, &'static u8)> = Some((7, &3)); static T: [Option<(&'static str, u8)>; 2] = [Some(("ab", 4)), None]; const A: Option<[&'static u8; 2]> = Some([&5, &6]);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(C.map_or(0, |(a, b)| (*a * 10 + *b) as u32)); print_u32(black_box(D).map_or(0, |(a, b)| a as u32 * 10 + *b as u32)); print_u32(T[black_box(0)].map_or(0, |(s, n)| s.len() as u32 * 10 + n as u32)); print_bool(T[black_box(1)].is_none()); print_u32(black_box(A).map_or(0, |a| (*a[0] * 10 + *a[1]) as u32));
}
