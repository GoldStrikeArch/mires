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
    print_u32(A.id * 10 + B.id);
}
