#![no_std]
#![no_main]
#![allow(unused, dead_code, warnings)]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
include!("vop_i8_2_body.rs");
