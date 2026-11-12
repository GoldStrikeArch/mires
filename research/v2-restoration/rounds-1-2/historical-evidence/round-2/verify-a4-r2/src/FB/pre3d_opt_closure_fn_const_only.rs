#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
const OF: Option<fn(u32) -> u32> = Some(|x| x + 7);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(OF.map_or(0, |f| f(1)));
}
