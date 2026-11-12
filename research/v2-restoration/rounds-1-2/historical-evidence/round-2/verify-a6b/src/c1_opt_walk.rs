#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
struct Node { id: u32, next: Option<&'static Node> }
static A: Node = Node { id: 1, next: Some(&B) };
static B: Node = Node { id: 2, next: Some(&A) };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut n = &A; let mut t = 0;
    for _ in 0..4 { t = t * 10 + n.id; n = n.next.unwrap(); }
    print_u32(t);
    print_bool(core::ptr::eq(A.next.unwrap().next.unwrap(), &A));
}
