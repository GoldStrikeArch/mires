#![no_std]
#![no_main]

#[derive(Clone, Copy)]
struct Pair {
    high: u32,
    low: u32,
}

#[inline(never)]
fn make_pair(seed: u32) -> Pair {
    Pair {
        high: seed.wrapping_mul(3).wrapping_add(1),
        low: seed ^ 0xa5a5_5a5a,
    }
}

#[inline(never)]
fn result_pair(seed: u32) -> Pair {
    Pair {
        high: seed.wrapping_add(9),
        low: seed.wrapping_mul(7),
    }
}

#[inline(never)]
#[no_mangle]
fn step(pair: &mut Pair, value: u32) {
    pair.high = pair.high.wrapping_mul(5).wrapping_add(value);
    pair.low = pair.low.rotate_left(7) ^ pair.high;
}

#[inline(never)]
fn fold_slice(bytes: &[u8]) -> u32 {
    let mut value = 0x811c_9dc5_u32;
    let mut index = 0;
    while index < bytes.len() {
        value = (value ^ u32::from(bytes[index])).wrapping_mul(0x0100_0193);
        index += 1;
    }
    value
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub fn cross_crate_physical_checksum() -> u32 {
    let mut pair = make_pair(17);
    let mut result_sum = 0_u32;
    for value in [3_u32, 5, 8, 13] {
        step(&mut pair, value);
        let result = result_pair(value.wrapping_add(23));
        result_sum = result_sum
            .wrapping_add(result.high)
            .wrapping_add(result.low);
    }
    let bytes = [1_u8, 2, 3, 5, 8, 13, 21, 34, 55];
    pair.high
        .wrapping_add(pair.low)
        .wrapping_add(result_sum)
        .wrapping_add(fold_slice(&bytes))
        .wrapping_add(fold_slice(&bytes[2..8]))
        .wrapping_add(fold_slice(&bytes))
        .wrapping_add(fold_slice(&bytes))
        .wrapping_add(fold_slice(&bytes))
}

#[no_mangle]
pub fn rust_entry() {}
