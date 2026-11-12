#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct W(&'static W);
static W1: W = W(&W2);
static W2: W = W(&W1);
static WS: W = W(&WS);
struct One { r: &'static One }
static O1: One = One { r: &O2 };
static O2: One = One { r: &O1 };
static OS: One = One { r: &OS };
struct T(u32, &'static T, u64);
static T1: T = T(1, &T2, 10);
static T2: T = T(2, &T1, 20);
struct Pn { id: u32, t: &'static (u32, &'static Pn) }
static TP: (u32, &'static Pn) = (7, &PN);
static PN: Pn = Pn { id: 8, t: &TP };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(core::ptr::eq(W1.0, &W2));
    print_bool(core::ptr::eq(W1.0.0, &W1));
    print_bool(core::ptr::eq(W2.0.0.0, &W1));
    print_bool(core::ptr::eq(WS.0, &WS));
    print_bool(core::ptr::eq(WS.0.0.0, &WS));
    print_bool(core::ptr::eq(O1.r, &O2));
    print_bool(core::ptr::eq(O1.r.r, &O1));
    print_bool(core::ptr::eq(OS.r.r, &OS));
    print_bool(core::ptr::eq(O1.r, &O1));
    print_u32(T1.1.0 * 10 + T1.1.1.0);
    print_u64(T1.2 + T1.1.2 * 3);
    print_bool(core::ptr::eq(T2.1, &T1));
    print_u32(TP.0 * 100 + TP.1.id * 10 + PN.t.0);
    print_bool(core::ptr::eq(TP.1, &PN));
    print_bool(core::ptr::eq(PN.t, &TP));
    print_bool(core::ptr::eq(PN.t.1.t, &TP));
}
