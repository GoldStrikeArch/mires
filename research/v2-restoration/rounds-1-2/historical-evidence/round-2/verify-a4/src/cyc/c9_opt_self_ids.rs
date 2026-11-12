#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
struct Node { id: u32, next: Option<&'static Node> }
static S: Node = Node { id: 7, next: Some(&S) };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(S.id);
}
