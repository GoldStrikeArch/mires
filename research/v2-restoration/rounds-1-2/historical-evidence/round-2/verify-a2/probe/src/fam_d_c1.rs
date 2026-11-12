#![no_std]
#![no_main]
#![allow(unused, dead_code, warnings)]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
include!("fam_d_c1_body.rs");
