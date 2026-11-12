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
const C: Named<&'static u32> = Named { z: (), inner: &TARGET };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(*C.inner); print_u32(*black_box(C).inner);
}
