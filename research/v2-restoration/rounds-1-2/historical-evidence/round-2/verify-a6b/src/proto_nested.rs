#![no_std]
#![no_main]
#![allow(dead_code, unused, non_snake_case)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// A cycle of statics whose members hold a `__proto__` field one level DOWN, in an inline struct.
// The member's own literal has no `__proto__` key of its own, so `Object.assign` copies every
// property it has; the nested literal `{ __proto__: B, k: 5 }` takes B as its prototype and reads
// back as `inner.__proto__ === B` (the printer gap that happens to work for a reference).
struct Inner { __proto__: &'static Outer, k: u32 }
struct Outer { id: u32, inner: Inner }
static A: Outer = Outer { id: 1, inner: Inner { __proto__: &B, k: 5 } };
static B: Outer = Outer { id: 2, inner: Inner { __proto__: &A, k: 6 } };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(A.inner.__proto__.id);
    print_u32(B.inner.__proto__.inner.__proto__.id);
    print_bool(core::ptr::eq(A.inner.__proto__, &B));
    print_u32(A.inner.k + B.inner.k * 10 + A.inner.__proto__.inner.k * 100);
}
