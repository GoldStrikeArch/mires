#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused, non_snake_case)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// No cycle. A field named `__proto__` holding a scalar is already wrong on main; holding a
// reference it works by accident (the literal sets the prototype and the read returns it).
struct N { id: u32, __proto__: u32 }
static NC1: N = N { id: 21, __proto__: 5 };
struct P { id: u32, __proto__: &'static N }
static PR: P = P { id: 1, __proto__: &NC1 };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(PR.__proto__.id);
    print_u32(NC1.__proto__);
}
