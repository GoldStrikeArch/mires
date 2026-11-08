#![no_std]
#![no_main]

extern crate cross_crate_physical_provider;

use cross_crate_physical_provider::{fold_slice, make_pair, result_pair, step};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub fn cross_crate_physical_alt_checksum() -> u32 {
    let mut pair = make_pair(2);
    let mut result_sum = 0_u32;
    for value in [1_u32, 4] {
        step(&mut pair, value);
        let result = result_pair(value.wrapping_add(7));
        result_sum = result_sum
            .wrapping_add(result.high)
            .wrapping_add(result.low);
    }
    let bytes = [2_u8, 4, 8, 16, 32, 64];
    pair.high
        .wrapping_add(pair.low)
        .wrapping_add(result_sum)
        .wrapping_add(fold_slice(&bytes))
}

#[no_mangle]
pub fn rust_entry() {}
