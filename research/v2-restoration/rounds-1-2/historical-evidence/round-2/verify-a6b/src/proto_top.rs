#![no_std]
#![no_main]
#![allow(dead_code, unused, non_snake_case)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// A cycle of statics whose members hold a `__proto__` field at the top level: what A6b refuses.
struct P { id: u32, __proto__: &'static P }
static CY1: P = P { id: 11, __proto__: &CY2 };
static CY2: P = P { id: 12, __proto__: &CY1 };
// A self reference through the same field.
static ME: P = P { id: 13, __proto__: &ME };

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(CY1.__proto__.id);
    print_u32(CY1.__proto__.__proto__.id);
    print_bool(core::ptr::eq(CY2.__proto__, &CY1));
    print_u32(ME.__proto__.__proto__.id);
}
