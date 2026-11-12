#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
const ROUNDS: u32 = 3_100_000;
include!("host2_body.rs");
