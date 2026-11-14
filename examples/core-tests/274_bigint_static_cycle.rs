//! BigInt constants in statics that name each other, beside a function that uses the same values.
//!
//! On `js-ecma-target=2018` the JIR lane names each distinct BigInt constant of a function by one
//! module-level binding (`jsc-link/src/lower/bigint.rs`); a static's initializer runs once and
//! keeps its inline constants. A cycle of statics is printed as every member declared empty and
//! then filled in (`jsc-link/src/link/statics.rs`): `A` and `B` name each other and hold 64-bit
//! keys, `S` names itself and holds `i64::MIN`, and `INTO`, outside the cycle, points into it and
//! repeats one of its keys. `walk` reads the keys around the cycle in a loop, with a constant
//! rotate amount and a seed that are bindings, and the values must agree whichever spelling
//! produced them.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct Node {
    key: u64,
    next: &'static Node,
}
static A: Node = Node { key: 0x0123_4567_89AB_CDEF, next: &B };
static B: Node = Node { key: u64::MAX - 5, next: &A };

struct Me {
    key: i64,
    me: &'static Me,
}
static S: Me = Me { key: i64::MIN, me: &S };

static INTO: (&Node, u64) = (&B, 0x0123_4567_89AB_CDEF);

#[inline(never)]
fn walk(start: &'static Node, steps: u32) -> u64 {
    let mut node = start;
    let mut acc = 0x9E37_79B9_7F4A_7C15u64;
    for _ in 0..steps {
        acc = acc.rotate_left(7) ^ node.key;
        node = node.next;
    }
    acc
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u64(A.key);
    print_u64(A.next.key);
    print_u64(walk(&A, 5));
    print_bool(core::ptr::eq(A.next.next, &A));
    print_i64(S.me.me.key);
    print_bool(core::ptr::eq(S.me, &S));
    print_u64(INTO.0.key ^ INTO.1);
    print_bool(core::ptr::eq(INTO.0.next, &A));
}
