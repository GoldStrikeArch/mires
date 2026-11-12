#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
const ROUNDS: u32 = 1000;
include!("host_body.rs");
