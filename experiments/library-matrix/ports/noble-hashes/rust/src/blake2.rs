//! BLAKE2b and BLAKE2s from @noble/hashes 2.4.0, with a pending final block.
use alloc::vec::Vec;

const IV_HIGH: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
const IV_LOW: [u32; 8] = [
    0xf3bcc908, 0x84caa73b, 0xfe94f82b, 0x5f1d36f1, 0xade682d1, 0x2b3e6c1f, 0xfb41bd6b, 0x137e2179,
];
const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

#[derive(Clone, Copy)]
struct Word {
    lo: u32,
    hi: u32,
}

const ZERO: Word = Word { lo: 0, hi: 0 };

impl Word {
    fn add(self, rhs: Self) -> Self {
        let (lo, carry) = self.lo.overflowing_add(rhs.lo);
        Self {
            lo,
            hi: self.hi.wrapping_add(rhs.hi).wrapping_add(carry as u32),
        }
    }

    fn xor(self, rhs: Self) -> Self {
        Self {
            lo: self.lo ^ rhs.lo,
            hi: self.hi ^ rhs.hi,
        }
    }

    fn rotate(self, bits: u32, wide: bool) -> Self {
        if !wide {
            return Self {
                lo: self.lo.rotate_right(bits),
                hi: 0,
            };
        }
        if bits == 32 {
            return Self {
                lo: self.hi,
                hi: self.lo,
            };
        }
        let (lo, hi, n) = if bits > 32 {
            (self.hi, self.lo, bits - 32)
        } else {
            (self.lo, self.hi, bits)
        };
        Self {
            lo: (lo >> n) | (hi << (32 - n)),
            hi: (hi >> n) | (lo << (32 - n)),
        }
    }
}

fn iv(i: usize, wide: bool) -> Word {
    if wide {
        Word {
            lo: IV_LOW[i],
            hi: IV_HIGH[i],
        }
    } else {
        Word {
            lo: IV_HIGH[i],
            hi: 0,
        }
    }
}

fn load32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

fn load(buf: &[u8], i: usize, wide: bool) -> Word {
    if wide {
        Word {
            lo: load32(buf, i * 8),
            hi: load32(buf, i * 8 + 4),
        }
    } else {
        Word {
            lo: load32(buf, i * 4),
            hi: 0,
        }
    }
}

fn mix(v: &mut [Word], a: usize, b: usize, c: usize, d: usize, x: Word, y: Word, wide: bool) {
    let (r1, r2, r3, r4) = if wide {
        (32, 24, 16, 63)
    } else {
        (16, 12, 8, 7)
    };
    v[a] = v[a].add(v[b]).add(x);
    v[d] = v[d].xor(v[a]).rotate(r1, wide);
    v[c] = v[c].add(v[d]);
    v[b] = v[b].xor(v[c]).rotate(r2, wide);
    v[a] = v[a].add(v[b]).add(y);
    v[d] = v[d].xor(v[a]).rotate(r3, wide);
    v[c] = v[c].add(v[d]);
    v[b] = v[b].xor(v[c]).rotate(r4, wide);
}

fn compress(h: &mut [Word], block: &[u8], count: Word, last: bool, wide: bool) {
    let mut v = [ZERO; 16];
    let mut msg = [ZERO; 16];
    for i in 0..8 {
        v[i] = h[i];
        v[i + 8] = iv(i, wide);
    }
    for i in 0..16 {
        msg[i] = load(block, i, wide);
    }
    v[12].lo ^= count.lo;
    if wide {
        v[12].hi ^= count.hi;
    } else {
        v[13].lo ^= count.hi;
    }
    if last {
        v[14].lo = !v[14].lo;
        v[14].hi = !v[14].hi;
    }
    let rounds = if wide { 12 } else { 10 };
    for r in 0..rounds {
        let sigma = &SIGMA[r % 10];
        for i in 0..4 {
            mix(
                &mut v,
                i,
                i + 4,
                i + 8,
                i + 12,
                msg[sigma[2 * i]],
                msg[sigma[2 * i + 1]],
                wide,
            );
        }
        for i in 0..4 {
            mix(
                &mut v,
                i,
                (i + 1) % 4 + 4,
                (i + 2) % 4 + 8,
                (i + 3) % 4 + 12,
                msg[sigma[8 + 2 * i]],
                msg[sigma[9 + 2 * i]],
                wide,
            );
        }
    }
    for i in 0..8 {
        h[i] = h[i].xor(v[i]).xor(v[i + 8]);
    }
}

pub struct Blake2 {
    h: [Word; 8],
    pending: [u8; 128],
    pos: usize,
    count: Word,
    wide: bool,
    out_len: usize,
}

impl Blake2 {
    pub fn new(wide: bool, dk_len: usize, key: &[u8], salt: &[u8], personalization: &[u8]) -> Self {
        let mut state = Self {
            h: [ZERO; 8],
            pending: [0; 128],
            pos: 0,
            count: ZERO,
            wide,
            out_len: dk_len,
        };
        for i in 0..8 {
            state.h[i] = iv(i, wide);
        }
        state.h[0].lo ^= 0x01010000 ^ ((key.len() as u32) << 8) ^ dk_len as u32;
        for i in 0..2 {
            if !salt.is_empty() {
                state.h[i + 4] = state.h[i + 4].xor(load(salt, i, wide));
            }
            if !personalization.is_empty() {
                state.h[i + 6] = state.h[i + 6].xor(load(personalization, i, wide));
            }
        }
        if !key.is_empty() {
            state.pending[..key.len()].copy_from_slice(key);
            state.pos = state.block_len();
            state.count.lo = state.pos as u32;
        }
        state
    }

    fn block_len(&self) -> usize {
        if self.wide {
            128
        } else {
            64
        }
    }

    pub fn update(&mut self, mut input: &[u8]) {
        let block_len = self.block_len();
        while !input.is_empty() {
            if self.pos == block_len {
                compress(&mut self.h, &self.pending, self.count, false, self.wide);
                self.pos = 0;
            }
            let take = core::cmp::min(input.len(), block_len - self.pos);
            self.count = self.count.add(Word {
                lo: take as u32,
                hi: 0,
            });
            if self.pos == 0 && take == block_len && input.len() > block_len {
                compress(&mut self.h, input, self.count, false, self.wide);
            } else {
                self.pending[self.pos..self.pos + take].copy_from_slice(&input[..take]);
                self.pos += take;
            }
            input = &input[take..];
        }
    }

    pub fn digest(mut self) -> Vec<u8> {
        self.pending[self.pos..].fill(0);
        compress(&mut self.h, &self.pending, self.count, true, self.wide);
        let stride = if self.wide { 8 } else { 4 };
        let mut bytes = Vec::with_capacity(self.out_len);
        for i in 0..self.out_len {
            let word = self.h[i / stride];
            let limb = if i % stride >= 4 { word.hi } else { word.lo };
            bytes.push((limb >> ((i % 4) * 8)) as u8);
        }
        bytes
    }
}
