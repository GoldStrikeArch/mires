//! One-element slices of statics inside static cycles (needs main's cf80163 for the cycles).
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr;
use core::slice::from_ref;

struct Node { id: u32, peers: &'static [Node] }
static A: Node = Node { id: 1, peers: from_ref(&B) };
static B: Node = Node { id: 2, peers: from_ref(&A) };
static SELF: Node = Node { id: 3, peers: from_ref(&SELF) };
struct ONode { id: u32, next: Option<&'static [ONode]> }
static X: ONode = ONode { id: 10, next: Some(from_ref(&Y)) };
static Y: ONode = ONode { id: 20, next: Some(from_ref(&Z)) };
static Z: ONode = ONode { id: 30, next: Some(from_ref(&X)) };
// a slice outside the cycle naming a member, declared before and after
static ONE_A: &[Node] = from_ref(&A);
static ZZ_ONE_B: &[Node] = from_ref(&B);
// a cycle through a thin reference and a slice
struct M { id: u32, thin: &'static M2 }
struct M2 { id: u32, back: &'static [M] }
static MA: M = M { id: 5, thin: &MB };
static MB: M2 = M2 { id: 6, back: from_ref(&MA) };
// a table static in a cycle with its element's slice
struct T { id: u32, table: &'static [&'static [T]; 2] }
static T0: T = T { id: 40, table: &TABLE };
static TABLE: [&[T]; 2] = [from_ref(&T0), from_ref(&T0)];

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(A.peers[0].id); print_u32(B.peers[0].id); print_u32(A.peers[0].peers[0].id);
    print_bool(ptr::eq(A.peers.as_ptr(), &B)); print_bool(ptr::eq(B.peers.as_ptr(), &A));
    print_bool(ptr::eq(&A.peers[0].peers[0], &A));
    print_usize(A.peers.len() + B.peers.len());
    print_u32(SELF.peers[0].peers[0].peers[black_box(0)].id); print_bool(ptr::eq(SELF.peers.as_ptr(), &SELF));
    let mut n = &X; let mut s = 0;
    for _ in 0..7 { s = s * 2 + n.id; n = &n.next.unwrap()[0]; }
    print_u32(s);
    print_bool(ptr::eq(X.next.unwrap().as_ptr(), &Y)); print_bool(ptr::eq(Z.next.unwrap().as_ptr(), &X));
    print_bool(ptr::eq(ONE_A.as_ptr(), &A)); print_bool(ptr::eq(ONE_A.as_ptr(), B.peers.as_ptr()));
    print_bool(ptr::eq(ZZ_ONE_B.as_ptr(), A.peers.as_ptr())); print_u32(ONE_A[0].peers[0].id + ZZ_ONE_B[0].peers[0].id);
    print_u32(MA.thin.back[0].id + MB.back[0].thin.id); print_bool(ptr::eq(MB.back.as_ptr(), &MA));
    print_u32(T0.table[1][0].id); print_bool(ptr::eq(TABLE[0].as_ptr(), &T0)); print_bool(ptr::eq(T0.table, &TABLE));
    print_bool(ptr::eq(A.peers.as_ptr(), from_ref(black_box(&B)).as_ptr()));
}
