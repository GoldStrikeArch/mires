#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
include!("heap_cycle3_body.rs");
