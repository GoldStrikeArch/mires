#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused, non_snake_case)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// A field named `__proto__`: in a JavaScript object literal `{ __proto__: X }` sets the prototype
// rather than an own property, and `Object.assign` does not copy a prototype.
struct P { id: u32, __proto__: &'static P }
// no cycle
static LEAFP: P = P { id: 1, __proto__: &LEAFP2 };
static LEAFP2: P = P { id: 2, __proto__: &LEAFP3 };
struct Q { id: u32 }
static LEAFP3: P = P { id: 3, __proto__: &CY1 };
// cycle
static CY1: P = P { id: 11, __proto__: &CY2 };
static CY2: P = P { id: 12, __proto__: &CY1 };
struct N { id: u32, __proto__: u32, next: &'static N }
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
