#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
struct Node { id: u32, next: Option<&'static Node> }
static A: Node = Node { id: 1, next: Some(&B) };
static B: Node = Node { id: 2, next: None };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(A.id * 10 + B.id);
    print_u32(A.next.map_or(0, |n| n.id));
    print_bool(B.next.is_none());
    print_bool(core::ptr::eq(A.next.unwrap(), &B));
}
