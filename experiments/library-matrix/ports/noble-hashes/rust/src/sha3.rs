//! SHA3-256 and Keccak-256 from @noble/hashes 2.4.0 (MIT).
//! Each 64-bit Keccak lane is represented by two little-endian 32-bit words.

use alloc::vec::Vec;

const RATE: usize = 136;
const PI: [usize; 24] = [
    20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2,
];
const ROTATIONS: [u32; 24] = [
    1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
];
const IOTA: [(u32, u32); 24] = [
    (0x00000001, 0x00000000),
    (0x00008082, 0x00000000),
    (0x0000808a, 0x80000000),
    (0x80008000, 0x80000000),
    (0x0000808b, 0x00000000),
    (0x80000001, 0x00000000),
    (0x80008081, 0x80000000),
    (0x00008009, 0x80000000),
    (0x0000008a, 0x00000000),
    (0x00000088, 0x00000000),
    (0x80008009, 0x00000000),
    (0x8000000a, 0x00000000),
    (0x8000808b, 0x00000000),
    (0x0000008b, 0x80000000),
    (0x00008089, 0x80000000),
    (0x00008003, 0x80000000),
    (0x00008002, 0x80000000),
    (0x00000080, 0x80000000),
    (0x0000800a, 0x00000000),
    (0x8000000a, 0x80000000),
    (0x80008081, 0x80000000),
    (0x00008080, 0x80000000),
    (0x80000001, 0x00000000),
    (0x80008008, 0x80000000),
];

fn permute(s: &mut [u32; 50]) {
    let mut columns = [0u32; 10];
    for &(rc_low, rc_high) in &IOTA {
        // Theta: mix each column with its two neighbors.
        for x in 0..10 {
            columns[x] = s[x] ^ s[x + 10] ^ s[x + 20] ^ s[x + 30] ^ s[x + 40];
        }
        for col in 0..5 {
            let x = 2 * col;
            let left = (x + 8) % 10;
            let right = (x + 2) % 10;
            let lo = columns[left] ^ ((columns[right] << 1) | (columns[right + 1] >> 31));
            let hi = columns[left + 1] ^ ((columns[right + 1] << 1) | (columns[right] >> 31));
            for y in 0..5 {
                s[10 * y + x] ^= lo;
                s[10 * y + x + 1] ^= hi;
            }
        }
        // Rho and Pi form one cycle through the 24 non-origin lanes.
        let mut lo = s[2];
        let mut hi = s[3];
        for t in 0..24 {
            let shift = ROTATIONS[t];
            let offset = PI[t];
            let (a, b) = if shift > 32 { (hi, lo) } else { (lo, hi) };
            let k = shift & 31; // all rotation offsets are nonzero modulo 32
            lo = s[offset];
            hi = s[offset + 1];
            s[offset] = (a << k) | (b >> (32 - k));
            s[offset + 1] = (b << k) | (a >> (32 - k));
        }
        // Chi: save the original first two lanes before changing a row.
        for i in 0..5 {
            let y = i * 10;
            let (b0, b1, b2, b3) = (s[y], s[y + 1], s[y + 2], s[y + 3]);
            for x in 0..6 {
                s[y + x] ^= !s[y + x + 2] & s[y + x + 4];
            }
            s[y + 6] ^= !s[y + 8] & b0;
            s[y + 7] ^= !s[y + 9] & b1;
            s[y + 8] ^= !b0 & b2;
            s[y + 9] ^= !b1 & b3;
        }
        s[0] ^= rc_low;
        s[1] ^= rc_high;
    }
}

pub struct Keccak {
    state: [u32; 50],
    pos: usize,
    suffix: u8,
}

impl Keccak {
    pub fn new(legacy: bool) -> Self {
        Self {
            state: [0; 50],
            pos: 0,
            suffix: if legacy { 1 } else { 6 },
        }
    }

    pub fn update(&mut self, input: &[u8]) {
        let mut offset = 0;
        while offset < input.len() {
            if self.pos == 0 && input.len() - offset >= RATE {
                for i in 0..RATE / 4 {
                    let p = offset + 4 * i;
                    let word =
                        u32::from_le_bytes([input[p], input[p + 1], input[p + 2], input[p + 3]]);
                    self.state[i] ^= word;
                }
                offset += RATE;
                permute(&mut self.state);
            } else {
                let take = (RATE - self.pos).min(input.len() - offset);
                for &byte in &input[offset..offset + take] {
                    self.state[self.pos / 4] ^= (byte as u32) << (8 * (self.pos % 4));
                    self.pos += 1;
                }
                offset += take;
                if self.pos == RATE {
                    permute(&mut self.state);
                    self.pos = 0;
                }
            }
        }
    }

    pub fn digest(mut self) -> Vec<u8> {
        self.state[self.pos / 4] ^= (self.suffix as u32) << (8 * (self.pos % 4));
        self.state[RATE / 4 - 1] ^= 0x80000000;
        permute(&mut self.state);
        let mut out = Vec::with_capacity(32);
        for &word in &self.state[..8] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }
}

pub fn hash(input: &[u8], legacy: bool) -> Vec<u8> {
    let mut state = Keccak::new(legacy);
    state.update(input);
    state.digest()
}
