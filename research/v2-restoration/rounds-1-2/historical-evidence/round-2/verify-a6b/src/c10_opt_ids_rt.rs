#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
struct Node { id: u32, next: Option<&'static Node> }
static A: Node = Node { id: 1, next: Some(&B) };
static B: Node = Node { id: 2, next: Some(&A) };
#[inline(never)] fn pick(i: usize) -> &'static Node { if core::hint::black_box(i) == 0 { &A } else { &B } }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(pick(0).id * 10 + pick(1).id);
}
