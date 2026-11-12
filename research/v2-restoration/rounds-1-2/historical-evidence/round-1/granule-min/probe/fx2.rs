#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
include!("fx2_body.rs");
