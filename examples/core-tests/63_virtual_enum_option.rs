//! `Option<(u32, u32)>` built and matched in the same function.
//!
//! Two shapes sit here. The option `pick` builds under a runtime condition is matched at once,
//! so both arms' values meet at a block parameter that immediately switches on its discriminant:
//! the jump-threading retargets each creation edge to its arm, each site then has one variant,
//! and the escape analysis scalarizes both. The option `fold` builds *inside* a loop body and
//! matches in that same body with one variant at each creation site, so it is scalarized and the
//! loop body builds nothing.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[inline(never)]
fn pick(seed: u32) -> u32 {
    let option = if seed > 0x8000_0000 {
        Some((seed, 1))
    } else {
        None
    };
    match option {
        Some((high, low)) => high.wrapping_add(low),
        None => seed ^ 0xff,
    }
}

#[inline(never)]
fn fold(bytes: &[u8; 8]) -> u32 {
    let mut acc = 0x9e37_79b9u32;
    let mut index = 0;
    while index < 8 {
        let byte = bytes[index];
        let step = Some((byte as u32, acc));
        acc = match step {
            Some((high, low)) => high.wrapping_mul(0x85eb_ca6b) ^ low,
            None => acc.wrapping_add(byte as u32),
        };
        index += 1;
    }
    acc
}

#[no_mangle]
pub fn rust_entry() {
    print_u32(pick(3));
    print_u32(pick(0x9000_0000));
    print_u32(fold(&[1, 2, 3, 4, 5, 6, 7, 8]));
}
