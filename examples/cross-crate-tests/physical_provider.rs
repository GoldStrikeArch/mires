#![no_std]

#[derive(Clone, Copy)]
pub struct Pair {
    pub high: u32,
    pub low: u32,
}

#[inline(never)]
pub fn make_pair(seed: u32) -> Pair {
    Pair {
        high: seed.wrapping_mul(3).wrapping_add(1),
        low: seed ^ 0xa5a5_5a5a,
    }
}

#[inline(never)]
pub fn result_pair(seed: u32) -> Pair {
    Pair {
        high: seed.wrapping_add(9),
        low: seed.wrapping_mul(7),
    }
}

#[inline(never)]
#[no_mangle]
pub fn step(pair: &mut Pair, value: u32) {
    pair.high = pair.high.wrapping_mul(5).wrapping_add(value);
    pair.low = pair.low.rotate_left(7) ^ pair.high;
}

#[inline(never)]
pub fn fold_slice(bytes: &[u8]) -> u32 {
    let mut value = 0x811c_9dc5_u32;
    let mut index = 0;
    while index < bytes.len() {
        value = (value ^ u32::from(bytes[index])).wrapping_mul(0x0100_0193);
        index += 1;
    }
    value
}
