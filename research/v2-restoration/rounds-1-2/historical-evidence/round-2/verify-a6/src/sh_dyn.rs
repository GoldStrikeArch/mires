#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

trait Tr: Sync { fn id(&self) -> u32; fn peer(&self) -> &'static dyn Tr; }
struct X { id: u32, peer: &'static dyn Tr }
struct Y { id: u32, k: u64, peer: &'static dyn Tr }
impl Tr for X { fn id(&self) -> u32 { self.id } fn peer(&self) -> &'static dyn Tr { self.peer } }
impl Tr for Y { fn id(&self) -> u32 { self.id + self.k as u32 } fn peer(&self) -> &'static dyn Tr { self.peer } }
static X1: X = X { id: 1, peer: &Y1 };
static Y1: Y = Y { id: 2, k: 40, peer: &X1 };
static XS: X = X { id: 3, peer: &XS };
static D: &dyn Tr = &X1;
struct F { id: u32, f: fn(&'static F) -> u32, next: &'static F, g: Option<fn(u32) -> u32> }
fn f1(x: &'static F) -> u32 { x.id + x.next.id * 10 }
fn f2(x: &'static F) -> u32 { x.id * 100 + (x.next.f)(x.next) }
fn dbl(x: u32) -> u32 { x * 2 }
static F1: F = F { id: 1, f: f1, next: &F2, g: Some(dbl) };
static F2: F = F { id: 2, f: f2, next: &F1, g: None };
static FP: fn(&'static F) -> u32 = f2;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(X1.peer.id() + X1.peer.peer().id() * 100);
    print_u32(Y1.peer().peer().peer().id());
    print_u32(XS.peer().peer().id());
    print_u32(D.peer().id());
    print_bool(core::ptr::eq(D as *const dyn Tr as *const u8, &X1 as *const X as *const u8));
    print_bool(core::ptr::eq(X1.peer as *const dyn Tr as *const u8, &Y1 as *const Y as *const u8));
    print_u32((F1.f)(&F1));
    print_u32((F2.f)(&F2));
    print_u32(FP(&F1));
    print_u32(F2.next.g.map(|g| g(21)).unwrap_or(0) + F1.next.g.map(|g| g(1)).unwrap_or(5));
    print_bool(core::ptr::eq(F1.next.next, &F1));
}
