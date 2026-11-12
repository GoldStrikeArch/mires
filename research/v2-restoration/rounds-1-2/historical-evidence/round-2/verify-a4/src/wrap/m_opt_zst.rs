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
struct Z; const C: Option<&'static Z> = Some(&Z); static ZS: () = (); static R: Option<&()> = Some(&ZS);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(C.is_some()); print_bool(black_box(C).is_some()); print_bool(R.is_some()); print_bool(black_box(&R).is_some());
}
