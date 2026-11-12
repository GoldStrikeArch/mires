#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
const ROUNDS: u32 = 3100000;
include!("cyc_body.rs");
