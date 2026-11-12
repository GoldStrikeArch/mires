#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
extern crate alloc;
use prelude::*;
include!("acc_vec_body.rs");
