//! Regression tests for `byte_views` reachability & escape analysis soundness:
//! - typed constant folding (`!0u8`, `-1i32 < 0`, `-1.0f64 < 0.0`) never kills live branches
//! - `&dyn Trait` vtable dispatch and `fn()` pointer indirection stay reachable
//! - `NonNull::new_unchecked` + `as_ptr` / `read` propagate taint instead of acting as sinks
//! - user enums named `Some`/`None` with custom `Drop` are never confused with `core::option::Option`

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr::NonNull;

trait ByteReader {
    fn read_low(&self, x: u32) -> (u8, u8);
}

struct LittleEndianReader;

impl ByteReader for LittleEndianReader {
    #[inline(never)]
    fn read_low(&self, x: u32) -> (u8, u8) {
        let p = (&x as *const u32) as *const u8;
        unsafe { (*p, *p.add(1)) }
    }
}

#[inline(never)]
fn read_via_fn_ptr(x: u32) -> (u8, u8) {
    let p = (&x as *const u32) as *const u8;
    unsafe { (*p, *p.add(1)) }
}

#[inline(never)]
fn call_dyn(r: &dyn ByteReader, x: u32) -> (u8, u8) {
    r.read_low(x)
}

#[inline(never)]
fn call_fn(f: fn(u32) -> (u8, u8), x: u32) -> (u8, u8) {
    f(x)
}

#[allow(dead_code)]
enum Sneaky {
    None,
    Some(*const u8),
}

impl Drop for Sneaky {
    fn drop(&mut self) {
        if let Sneaky::Some(p) = *self {
            unsafe {
                let b0 = *p;
                let b1 = *p.add(1);
                js_log_bool(b0 == 0x44 && b1 == 0x33);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. Typed constant folding guards: !0u8 == 255, -1i32 < 0, -1.0f64 < 0.0
    if (!0u8) == 255u8 {
        let x: u32 = black_box(0x5566_7788);
        let p = (&x as *const u32) as *const u8;
        unsafe {
            js_log_bool(*p == 0x88 && *p.add(1) == 0x77);
        }
    }
    if -1i32 < 0i32 {
        let x: u32 = black_box(0x1122_3344);
        let p = (&x as *const u32) as *const u8;
        unsafe {
            js_log_bool(*p == 0x44 && *p.add(1) == 0x33);
        }
    }
    if -1.0f64 < 0.0f64 {
        let x: u32 = black_box(0x5566_7788);
        let p = (&x as *const u32) as *const u8;
        unsafe {
            js_log_bool(*p == 0x88 && *p.add(1) == 0x77);
        }
    }

    // 2. &dyn Trait and fn() pointer reachability
    let reader = LittleEndianReader;
    let (d0, d1) = call_dyn(black_box(&reader), black_box(0x1122_3344));
    let (f0, f1) = call_fn(black_box(read_via_fn_ptr), black_box(0x5566_7788));
    unsafe {
        js_log_bool(d0 == 0x44 && d1 == 0x33 && f0 == 0x88 && f1 == 0x77);
    }

    // 3. NonNull::new_unchecked + as_ptr and NonNull::read
    let x: u32 = black_box(0x1122_3344);
    unsafe {
        let p = (&x as *const u32) as *mut u8;
        let nn = NonNull::new_unchecked(p);
        let q = nn.as_ptr();
        js_log_bool(*q == 0x44 && *q.add(1) == 0x33);
        let r0 = nn.read();
        let r1 = NonNull::new_unchecked(p.add(1)).read();
        js_log_bool(r0 == 0x44 && r1 == 0x33);
    }

    // 4. Custom user enum named Some/None with Drop reading bytes
    let y: u32 = black_box(0x1122_3344);
    let s = Sneaky::Some((&y as *const u32) as *const u8);
    drop(s);
}
