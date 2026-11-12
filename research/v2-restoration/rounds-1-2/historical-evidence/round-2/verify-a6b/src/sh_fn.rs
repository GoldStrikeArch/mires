#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct F { id: u32, f: fn(&'static F) -> u32, next: &'static F, g: Option<fn(u32) -> u32> }
fn f1(x: &'static F) -> u32 { x.id + x.next.id * 10 }
fn f2(x: &'static F) -> u32 { x.id * 100 + (x.next.f)(x.next) }
fn dbl(x: u32) -> u32 { x * 2 }
static F1: F = F { id: 1, f: f1, next: &F2, g: Some(dbl) };
static F2: F = F { id: 2, f: f2, next: &F1, g: None };
static FP: fn(&'static F) -> u32 = f2;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32((F1.f)(&F1));
    print_u32((F2.f)(&F2));
    print_u32(FP(&F1));
    print_u32(F2.next.g.map(|g| g(21)).unwrap_or(0) + F1.next.g.map(|g| g(1)).unwrap_or(5));
    print_bool(core::ptr::eq(F1.next.next, &F1));
}
