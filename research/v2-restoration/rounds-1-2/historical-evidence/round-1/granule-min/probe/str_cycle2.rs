#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
include!("str_cycle2_body.rs");
