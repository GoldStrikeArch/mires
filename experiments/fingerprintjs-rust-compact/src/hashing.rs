//! Unboxed MurmurHash3 x64 128. Four `u32` lanes, no `{high,low}` struct.

#[inline(always)]
fn lo(value: u32) -> u32 {
    value & 0xffff
}

#[inline(always)]
fn hi(value: u32) -> u32 {
    value >> 16
}

#[inline(always)]
fn u32_le(block: &[u8], offset: usize) -> u32 {
    u32::from(block[offset])
        | (u32::from(block[offset + 1]) << 8)
        | (u32::from(block[offset + 2]) << 16)
        | (u32::from(block[offset + 3]) << 24)
}

#[inline(always)]
fn add(ah: u32, al: u32, bh: u32, bl: u32) -> (u32, u32) {
    let (low, carry) = al.overflowing_add(bl);
    (ah.wrapping_add(bh).wrapping_add(u32::from(carry)), low)
}

#[inline(always)]
fn mul(ah: u32, al: u32, bh: u32, bl: u32) -> (u32, u32) {
    let m0 = hi(ah);
    let m1 = lo(ah);
    let m2 = hi(al);
    let m3 = lo(al);
    let n0 = hi(bh);
    let n1 = lo(bh);
    let n2 = hi(bl);
    let n3 = lo(bl);

    let mut o0 = 0_u32;
    let mut o1 = 0_u32;
    let mut o2 = 0_u32;
    let mut o3 = m3 * n3;
    o2 += o3 >> 16;
    o3 &= 0xffff;
    o2 += m2 * n3;
    o1 += o2 >> 16;
    o2 &= 0xffff;
    o2 += m3 * n2;
    o1 += o2 >> 16;
    o2 &= 0xffff;
    o1 += m1 * n3;
    o0 += o1 >> 16;
    o1 &= 0xffff;
    o1 += m2 * n2;
    o0 += o1 >> 16;
    o1 &= 0xffff;
    o1 += m3 * n1;
    o0 += o1 >> 16;
    o1 &= 0xffff;
    o0 = o0.wrapping_add(
        m0.wrapping_mul(n3)
            .wrapping_add(m1.wrapping_mul(n2))
            .wrapping_add(m2.wrapping_mul(n1))
            .wrapping_add(m3.wrapping_mul(n0)),
    ) & 0xffff;

    ((o0 << 16) | o1, (o2 << 16) | o3)
}

#[inline(always)]
fn rotl(ah: u32, al: u32, bits: u32) -> (u32, u32) {
    let bits = bits % 64;
    match bits {
        0 => (ah, al),
        32 => (al, ah),
        1..=31 => ((ah << bits) | (al >> (32 - bits)), (al << bits) | (ah >> (32 - bits))),
        _ => {
            let bits = bits - 32;
            ((al << bits) | (ah >> (32 - bits)), (ah << bits) | (al >> (32 - bits)))
        }
    }
}

#[inline(always)]
fn xor(ah: u32, al: u32, bh: u32, bl: u32) -> (u32, u32) {
    (ah ^ bh, al ^ bl)
}

#[inline(always)]
fn xor_byte(ah: u32, al: u32, byte: u8, shift: usize) -> (u32, u32) {
    let value = u32::from(byte);
    if shift < 32 {
        (ah, al ^ (value << shift))
    } else {
        (ah ^ (value << (shift - 32)), al)
    }
}

fn final_mix(ah: u32, al: u32) -> (u32, u32) {
    let (mut h, mut l) = xor(ah, al, 0, ah >> 1);
    (h, l) = mul(h, l, F1H, F1L);
    (h, l) = xor(h, l, 0, h >> 1);
    (h, l) = mul(h, l, F2H, F2L);
    xor(h, l, 0, h >> 1)
}

const F1H: u32 = 0xff51_afd7;
const F1L: u32 = 0xed55_8ccd;
const F2H: u32 = 0xc4ce_b9fe;
const F2L: u32 = 0x1a85_ec53;
const C1H: u32 = 0x87c3_7b91;
const C1L: u32 = 0x1142_53d5;
const C2H: u32 = 0x4cf5_ad43;
const C2L: u32 = 0x2745_937f;
const N1L: u32 = 0x52dc_e729;
const N2L: u32 = 0x3849_5ab5;

/// Computes FingerprintJS's MurmurHash3 x64 128-bit hash with seed zero.
pub fn x64hash128(input: &str) -> [u8; 32] {
    x64hash128_seed(input, 0)
}

/// Computes FingerprintJS's MurmurHash3 x64 128-bit hash with an explicit seed.
pub fn x64hash128_seed(input: &str, seed: u32) -> [u8; 32] {
    x64hash128_bytes(input.as_bytes(), seed)
}

/// Host string of a 128-bit hex digest. Avoids `str::from_utf8` (it panics in this backend).
pub fn hash_hex(digest: &[u8; 32]) -> mires_js::JsValue {
    let parts = crate::array();
    let mut index = 0;
    while index < 32 {
        crate::push(parts, mires_js::JsValue::erase(hex_digit(digest[index])));
        index += 1;
    }
    crate::call(parts, "join", &[mires_js::JsValue::erase("")])
}

fn hex_digit(byte: u8) -> &'static str {
    match byte {
        b'0' => "0",
        b'1' => "1",
        b'2' => "2",
        b'3' => "3",
        b'4' => "4",
        b'5' => "5",
        b'6' => "6",
        b'7' => "7",
        b'8' => "8",
        b'9' => "9",
        b'a' => "a",
        b'b' => "b",
        b'c' => "c",
        b'd' => "d",
        b'e' => "e",
        b'f' => "f",
        _ => "0",
    }
}

fn x64hash128_bytes(key: &[u8], seed: u32) -> [u8; 32] {
    let remainder = key.len() % 16;
    let bytes = key.len() - remainder;
    let length_low = key.len() as u32;
    let mut h1h = 0;
    let mut h1l = seed;
    let mut h2h = 0;
    let mut h2l = seed;

    let mut offset = 0;
    while offset < bytes {
        let mut k1h = u32_le(key, offset + 4);
        let mut k1l = u32_le(key, offset);
        let mut k2h = u32_le(key, offset + 12);
        let mut k2l = u32_le(key, offset + 8);

        (k1h, k1l) = mul(k1h, k1l, C1H, C1L);
        (k1h, k1l) = rotl(k1h, k1l, 31);
        (k1h, k1l) = mul(k1h, k1l, C2H, C2L);
        (h1h, h1l) = xor(h1h, h1l, k1h, k1l);
        (h1h, h1l) = rotl(h1h, h1l, 27);
        (h1h, h1l) = add(h1h, h1l, h2h, h2l);
        (h1h, h1l) = mul(h1h, h1l, 0, 5);
        (h1h, h1l) = add(h1h, h1l, 0, N1L);

        (k2h, k2l) = mul(k2h, k2l, C2H, C2L);
        (k2h, k2l) = rotl(k2h, k2l, 33);
        (k2h, k2l) = mul(k2h, k2l, C1H, C1L);
        (h2h, h2l) = xor(h2h, h2l, k2h, k2l);
        (h2h, h2l) = rotl(h2h, h2l, 31);
        (h2h, h2l) = add(h2h, h2l, h1h, h1l);
        (h2h, h2l) = mul(h2h, h2l, 0, 5);
        (h2h, h2l) = add(h2h, h2l, 0, N2L);

        offset += 16;
    }

    let tail = &key[bytes..];
    let mut k1h = 0;
    let mut k1l = 0;
    let mut k2h = 0;
    let mut k2l = 0;
    let mut tail_index = 0;
    while tail_index < tail.len() && tail_index < 8 {
        (k1h, k1l) = xor_byte(k1h, k1l, tail[tail_index], tail_index * 8);
        tail_index += 1;
    }
    while tail_index < tail.len() {
        (k2h, k2l) = xor_byte(k2h, k2l, tail[tail_index], (tail_index - 8) * 8);
        tail_index += 1;
    }
    if remainder > 8 {
        (k2h, k2l) = mul(k2h, k2l, C2H, C2L);
        (k2h, k2l) = rotl(k2h, k2l, 33);
        (k2h, k2l) = mul(k2h, k2l, C1H, C1L);
        (h2h, h2l) = xor(h2h, h2l, k2h, k2l);
    }
    if remainder > 0 {
        (k1h, k1l) = mul(k1h, k1l, C1H, C1L);
        (k1h, k1l) = rotl(k1h, k1l, 31);
        (k1h, k1l) = mul(k1h, k1l, C2H, C2L);
        (h1h, h1l) = xor(h1h, h1l, k1h, k1l);
    }

    (h1h, h1l) = xor(h1h, h1l, 0, length_low);
    (h2h, h2l) = xor(h2h, h2l, 0, length_low);
    (h1h, h1l) = add(h1h, h1l, h2h, h2l);
    (h2h, h2l) = add(h2h, h2l, h1h, h1l);
    (h1h, h1l) = final_mix(h1h, h1l);
    (h2h, h2l) = final_mix(h2h, h2l);
    (h1h, h1l) = add(h1h, h1l, h2h, h2l);
    (h2h, h2l) = add(h2h, h2l, h1h, h1l);

    let mut result = [0_u8; 32];
    write_hex(&mut result, 0, h1h);
    write_hex(&mut result, 8, h1l);
    write_hex(&mut result, 16, h2h);
    write_hex(&mut result, 24, h2l);
    result
}

fn write_hex(output: &mut [u8; 32], start: usize, value: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut shift = 0;
    while shift < 8 {
        output[start + shift] = HEX[((value >> ((7 - shift) * 4)) & 0xf) as usize];
        shift += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_text(digest: &[u8; 32]) -> alloc::string::String {
        let mut text = alloc::string::String::new();
        let mut index = 0;
        while index < 32 {
            text.push_str(hex_digit(digest[index]));
            index += 1;
        }
        text
    }

    #[test]
    fn matches_upstream_vectors() {
        assert_eq!(
            digest_text(&x64hash128("Hello, world, hi")),
            "9a66b4567d520770dc8eaf9a508ecf1b"
        );
        assert_eq!(
            digest_text(&x64hash128("Hello, world, h")),
            "4552c6409e0a7bd3b0f9eb318bb35f05"
        );
        assert_eq!(
            digest_text(&x64hash128("Hello, world, hi!")),
            "dce3e02d43da4d2374e84e484c566492"
        );
        assert_eq!(
            digest_text(&x64hash128("Hello, world, hi, Hello, world, hi")),
            "d49c261c833b671870b471c42df4dbf0"
        );
        assert_eq!(
            digest_text(&x64hash128("hello")),
            "cbd8a7b341bd9b025b1e906a48ae1d19"
        );
    }
}
