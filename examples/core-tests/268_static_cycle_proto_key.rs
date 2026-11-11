//! A cycle of statics holding a field named `__proto__` is refused at link time.
//!
//! A Rust field named `__proto__` prints as the object literal key `__proto__:`, which in
//! JavaScript sets the object's prototype instead of creating a property (a printer gap that
//! predates the cycle rewrite, outside a cycle as well). A cycle is initialized by copying each
//! member's literal onto an empty object with `Object.assign`, which copies own properties only,
//! so a member holding that key would silently lose the field. The link step refuses the cycle
//! instead (`jsc-link/src/link/statics.rs`); `268_static_cycle_proto_key.expect_fail` pins the
//! message. Natively the program prints `2 3 12 11 true 65`.

#![no_std]
#![no_main]
#![allow(non_snake_case)]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct P {
    id: u32,
    __proto__: &'static P,
}
// Not a cycle: a chain ending in the cycle below.
static LEAFP: P = P { id: 1, __proto__: &LEAFP2 };
static LEAFP2: P = P { id: 2, __proto__: &LEAFP3 };
static LEAFP3: P = P { id: 3, __proto__: &CY1 };
// A cycle whose members hold the key.
static CY1: P = P { id: 11, __proto__: &CY2 };
static CY2: P = P { id: 12, __proto__: &CY1 };

struct N {
    id: u32,
    __proto__: u32,
    next: &'static N,
}
static NC1: N = N { id: 21, __proto__: 5, next: &NC2 };
static NC2: N = N { id: 22, __proto__: 6, next: &NC1 };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(LEAFP.__proto__.id);
    print_u32(LEAFP.__proto__.__proto__.id);
    print_u32(CY1.__proto__.id);
    print_u32(CY1.__proto__.__proto__.id);
    print_bool(core::ptr::eq(CY2.__proto__, &CY1));
    print_u32(NC1.__proto__ + NC1.next.__proto__ * 10);
}
