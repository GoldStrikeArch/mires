//! SHA-256, SHA-512 and HMAC-SHA-256 from @noble/hashes 2.4.0.

use alloc::vec;
use alloc::vec::Vec;

#[rustfmt::skip]
const IV256: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[rustfmt::skip]
const IV512: [u32; 16] = [
    0x6a09e667, 0xf3bcc908, 0xbb67ae85, 0x84caa73b, 0x3c6ef372, 0xfe94f82b, 0xa54ff53a, 0x5f1d36f1,
    0x510e527f, 0xade682d1, 0x9b05688c, 0x2b3e6c1f, 0x1f83d9ab, 0xfb41bd6b, 0x5be0cd19, 0x137e2179,
];

#[rustfmt::skip]
const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[rustfmt::skip]
const K512_H: [u32; 80] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    0xca273ece, 0xd186b8c7, 0xeada7dd6, 0xf57d4f7f, 0x06f067aa, 0x0a637dc5, 0x113f9804, 0x1b710b35,
    0x28db77f5, 0x32caab7b, 0x3c9ebe0a, 0x431d67c4, 0x4cc5d4be, 0x597f299c, 0x5fcb6fab, 0x6c44198c,
];

#[rustfmt::skip]
const K512_L: [u32; 80] = [
    0xd728ae22, 0x23ef65cd, 0xec4d3b2f, 0x8189dbbc, 0xf348b538, 0xb605d019, 0xaf194f9b, 0xda6d8118,
    0xa3030242, 0x45706fbe, 0x4ee4b28c, 0xd5ffb4e2, 0xf27b896f, 0x3b1696b1, 0x25c71235, 0xcf692694,
    0x9ef14ad2, 0x384f25e3, 0x8b8cd5b5, 0x77ac9c65, 0x592b0275, 0x6ea6e483, 0xbd41fbd4, 0x831153b5,
    0xee66dfab, 0x2db43210, 0x98fb213f, 0xbeef0ee4, 0x3da88fc2, 0x930aa725, 0xe003826f, 0x0a0e6e70,
    0x46d22ffc, 0x5c26c926, 0x5ac42aed, 0x9d95b3df, 0x8baf63de, 0x3c77b2a8, 0x47edaee6, 0x1482353b,
    0x4cf10364, 0xbc423001, 0xd0f89791, 0x0654be30, 0xd6ef5218, 0x5565a910, 0x5771202a, 0x32bbd1b8,
    0xb8d2d0c8, 0x5141ab53, 0xdf8eeb99, 0xe19b48a8, 0xc5c95a63, 0xe3418acb, 0x7763e373, 0xd6b2b8a3,
    0x5defb2fc, 0x43172f60, 0xa1f0ab72, 0x1a6439ec, 0x23631e28, 0xde82bde9, 0xb2c67915, 0xe372532b,
    0xea26619c, 0x21c0c207, 0xcde0eb1e, 0xee6ed178, 0x72176fba, 0xa2c898a6, 0xbef90dae, 0x131c471b,
    0x23047d84, 0x40c72493, 0x15c9bebc, 0x9c100d4c, 0xcb3e42b6, 0xfc657e2a, 0x3ad6faec, 0x4a475817,
];

#[inline]
fn read_be(block: &[u8], at: usize) -> u32 {
    ((block[at] as u32) << 24)
        | ((block[at + 1] as u32) << 16)
        | ((block[at + 2] as u32) << 8)
        | block[at + 3] as u32
}

#[inline]
fn choice(a: u32, b: u32, c: u32) -> u32 {
    (a & b) ^ (!a & c)
}
#[inline]
fn majority(a: u32, b: u32, c: u32) -> u32 {
    (a & b) ^ (a & c) ^ (b & c)
}

fn compress256(state: &mut [u32], block: &[u8], w: &mut [u32]) {
    for i in 0..16 {
        w[i] = read_be(block, i * 4);
    }
    for i in 16..64 {
        let x = w[i - 15];
        let y = w[i - 2];
        let s0 = x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3);
        let s1 = y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10);
        w[i] = s1
            .wrapping_add(w[i - 7])
            .wrapping_add(s0)
            .wrapping_add(w[i - 16]);
    }
    let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
    let (mut e, mut f, mut g, mut h) = (state[4], state[5], state[6], state[7]);
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(choice(e, f, g))
            .wrapping_add(K256[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let t2 = s0.wrapping_add(majority(a, b, c));
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (i, word) in [a, b, c, d, e, f, g, h].into_iter().enumerate() {
        state[i] = state[i].wrapping_add(word);
    }
}

// SHA-512 uses high/low 32-bit lanes in the same order as noble's _u64 helpers.
#[derive(Copy, Clone)]
struct Word {
    hi: u32,
    lo: u32,
}
impl Word {
    #[inline]
    fn add(self, other: Self) -> Self {
        let lo = self.lo.wrapping_add(other.lo);
        Self {
            hi: self
                .hi
                .wrapping_add(other.hi)
                .wrapping_add((lo < self.lo) as u32),
            lo,
        }
    }
    #[inline]
    fn ror(self, n: u32) -> Self {
        let (h, l, r) = if n < 32 {
            (self.hi, self.lo, n)
        } else {
            (self.lo, self.hi, n - 32)
        };
        Self {
            hi: (h >> r) | (l << (32 - r)),
            lo: (l >> r) | (h << (32 - r)),
        }
    }
    #[inline]
    fn shr(self, n: u32) -> Self {
        Self {
            hi: self.hi >> n,
            lo: (self.lo >> n) | (self.hi << (32 - n)),
        }
    }
    #[inline]
    fn xor3(self, b: Self, c: Self) -> Self {
        Self {
            hi: self.hi ^ b.hi ^ c.hi,
            lo: self.lo ^ b.lo ^ c.lo,
        }
    }
}

fn compress512(state: &mut [u32], block: &[u8], wh: &mut [u32], wl: &mut [u32]) {
    for i in 0..16 {
        wh[i] = read_be(block, i * 8);
        wl[i] = read_be(block, i * 8 + 4);
    }
    for i in 16..80 {
        let x = Word {
            hi: wh[i - 15],
            lo: wl[i - 15],
        };
        let y = Word {
            hi: wh[i - 2],
            lo: wl[i - 2],
        };
        let s0 = x.ror(1).xor3(x.ror(8), x.shr(7));
        let s1 = y.ror(19).xor3(y.ror(61), y.shr(6));
        let sum = s0
            .add(s1)
            .add(Word {
                hi: wh[i - 7],
                lo: wl[i - 7],
            })
            .add(Word {
                hi: wh[i - 16],
                lo: wl[i - 16],
            });
        wh[i] = sum.hi;
        wl[i] = sum.lo;
    }
    // WORKAROUND(MIRES-020): a closure capturing state is refused at the call terminator.
    fn lane(state: &[u32], i: usize) -> Word {
        Word {
            hi: state[2 * i],
            lo: state[2 * i + 1],
        }
    }
    let (mut a, mut b, mut c, mut d) = (
        lane(state, 0),
        lane(state, 1),
        lane(state, 2),
        lane(state, 3),
    );
    let (mut e, mut f, mut g, mut h) = (
        lane(state, 4),
        lane(state, 5),
        lane(state, 6),
        lane(state, 7),
    );
    for i in 0..80 {
        let s1 = e.ror(14).xor3(e.ror(18), e.ror(41));
        let chi = Word {
            hi: choice(e.hi, f.hi, g.hi),
            lo: choice(e.lo, f.lo, g.lo),
        };
        let t1 = h
            .add(s1)
            .add(chi)
            .add(Word {
                hi: K512_H[i],
                lo: K512_L[i],
            })
            .add(Word {
                hi: wh[i],
                lo: wl[i],
            });
        let s0 = a.ror(28).xor3(a.ror(34), a.ror(39));
        let maj = Word {
            hi: majority(a.hi, b.hi, c.hi),
            lo: majority(a.lo, b.lo, c.lo),
        };
        h = g;
        g = f;
        f = e;
        e = d.add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.add(s0).add(maj);
    }
    for (i, word) in [a, b, c, d, e, f, g, h].into_iter().enumerate() {
        let sum = word.add(Word {
            hi: state[2 * i],
            lo: state[2 * i + 1],
        });
        state[2 * i] = sum.hi;
        state[2 * i + 1] = sum.lo;
    }
}

pub struct Sha2 {
    state: Vec<u32>,
    buffer: Vec<u8>,
    wh: Vec<u32>,
    wl: Vec<u32>,
    pos: usize,
    length: u64,
}

impl Sha2 {
    pub fn sha256() -> Self {
        Self {
            state: IV256.to_vec(),
            buffer: vec![0; 64],
            wh: vec![0; 64],
            wl: Vec::new(),
            pos: 0,
            length: 0,
        }
    }
    pub fn sha512() -> Self {
        Self {
            state: IV512.to_vec(),
            buffer: vec![0; 128],
            wh: vec![0; 80],
            wl: vec![0; 80],
            pos: 0,
            length: 0,
        }
    }

    fn process_buffer(&mut self) {
        if self.buffer.len() == 64 {
            compress256(&mut self.state, &self.buffer, &mut self.wh)
        } else {
            compress512(&mut self.state, &self.buffer, &mut self.wh, &mut self.wl)
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        let block = self.buffer.len();
        let mut at = 0;
        while at < data.len() {
            if self.pos == 0 && data.len() - at >= block {
                let full = &data[at..at + block];
                if block == 64 {
                    compress256(&mut self.state, full, &mut self.wh)
                } else {
                    compress512(&mut self.state, full, &mut self.wh, &mut self.wl)
                }
                at += block;
            } else {
                let take = (block - self.pos).min(data.len() - at);
                self.buffer[self.pos..self.pos + take].copy_from_slice(&data[at..at + take]);
                self.pos += take;
                at += take;
                if self.pos == block {
                    self.process_buffer();
                    self.pos = 0;
                }
            }
        }
        self.length += data.len() as u64;
    }

    pub fn digest(mut self) -> Vec<u8> {
        let block = self.buffer.len();
        self.buffer[self.pos] = 0x80;
        self.buffer[self.pos + 1..].fill(0);
        if self.pos + 1 > block - block / 8 {
            self.process_buffer();
            self.buffer.fill(0);
        }
        let bits = (self.length * 8).to_be_bytes();
        self.buffer[block - 8..].copy_from_slice(&bits);
        self.process_buffer();
        let mut out = Vec::with_capacity(self.state.len() * 4);
        for &word in &self.state {
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }
}

pub fn sha256(data: &[u8]) -> Vec<u8> {
    let mut h = Sha2::sha256();
    h.update(data);
    h.digest()
}
pub fn sha512(data: &[u8]) -> Vec<u8> {
    let mut h = Sha2::sha512();
    h.update(data);
    h.digest()
}

pub struct HmacSha256 {
    inner: Sha2,
    outer: Sha2,
}
impl HmacSha256 {
    pub fn new(key: &[u8]) -> Self {
        let mut pad = [0u8; 64];
        if key.len() > 64 {
            pad[..32].copy_from_slice(&sha256(key));
        } else {
            pad[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha2::sha256();
        for b in &mut pad {
            *b ^= 0x36;
        }
        inner.update(&pad);
        let mut outer = Sha2::sha256();
        for b in &mut pad {
            *b ^= 0x36 ^ 0x5c;
        }
        outer.update(&pad);
        pad.fill(0);
        Self { inner, outer }
    }
    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }
    pub fn digest(mut self) -> Vec<u8> {
        let inner = self.inner.digest();
        self.outer.update(&inner);
        self.outer.digest()
    }
}
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut h = HmacSha256::new(key);
    h.update(data);
    h.digest()
}
