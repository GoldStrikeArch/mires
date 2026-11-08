use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy)]
struct X64 {
    high: u32,
    low: u32,
}

impl X64 {
    const fn new(high: u32, low: u32) -> Self {
        Self { high, low }
    }

    fn add(&mut self, other: Self) {
        let (low, carry) = self.low.overflowing_add(other.low);
        self.high = self
            .high
            .wrapping_add(other.high)
            .wrapping_add(u32::from(carry));
        self.low = low;
    }

    fn multiply(&mut self, other: Self) {
        let m0 = self.high >> 16;
        let m1 = self.high & 0xffff;
        let m2 = self.low >> 16;
        let m3 = self.low & 0xffff;
        let n0 = other.high >> 16;
        let n1 = other.high & 0xffff;
        let n2 = other.low >> 16;
        let n3 = other.low & 0xffff;

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

        self.high = (o0 << 16) | o1;
        self.low = (o2 << 16) | o3;
    }

    fn rotate_left(&mut self, bits: u32) {
        let bits = bits % 64;
        let high = self.high;
        match bits {
            0 => {}
            32 => {
                self.high = self.low;
                self.low = high;
            }
            1..=31 => {
                self.high = (high << bits) | (self.low >> (32 - bits));
                self.low = (self.low << bits) | (high >> (32 - bits));
            }
            _ => {
                let bits = bits - 32;
                self.high = (self.low << bits) | (high >> (32 - bits));
                self.low = (high << bits) | (self.low >> (32 - bits));
            }
        }
    }

    fn xor(&mut self, other: Self) {
        self.high ^= other.high;
        self.low ^= other.low;
    }

    fn xor_byte(&mut self, byte: u8, shift: usize) {
        let value = u32::from(byte);
        if shift < 32 {
            self.low ^= value << shift;
        } else {
            self.high ^= value << (shift - 32);
        }
    }

    fn final_mix(&mut self) {
        self.xor(Self::new(0, self.high >> 1));
        self.multiply(F1);
        self.xor(Self::new(0, self.high >> 1));
        self.multiply(F2);
        self.xor(Self::new(0, self.high >> 1));
    }
}

const F1: X64 = X64::new(0xff51_afd7, 0xed55_8ccd);
const F2: X64 = X64::new(0xc4ce_b9fe, 0x1a85_ec53);
const C1: X64 = X64::new(0x87c3_7b91, 0x1142_53d5);
const C2: X64 = X64::new(0x4cf5_ad43, 0x2745_937f);
const M: X64 = X64::new(0, 5);
const N1: X64 = X64::new(0, 0x52dc_e729);
const N2: X64 = X64::new(0, 0x3849_5ab5);

/// Computes FingerprintJS's MurmurHash3 x64 128-bit hash with seed zero.
pub fn x64hash128(input: &str) -> String {
    x64hash128_seed(input, 0)
}

/// Computes FingerprintJS's MurmurHash3 x64 128-bit hash with an explicit seed.
pub fn x64hash128_seed(input: &str, seed: u32) -> String {
    x64hash128_bytes(input.as_bytes(), seed)
}

/// Hashes JavaScript UTF-16 code units with the same encoding as `TextEncoder`.
///
/// Unpaired surrogates become the UTF-8 replacement character, matching the
/// browser behavior that a Rust `str` cannot represent directly.
pub fn x64hash128_utf16(input: &[u16], seed: u32) -> String {
    x64hash128_bytes(&utf16_to_utf8(input), seed)
}

fn utf16_to_utf8(input: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(input.len());
    for character in char::decode_utf16(input.iter().copied()) {
        let character = character.unwrap_or(char::REPLACEMENT_CHARACTER);
        let mut encoded = [0; 4];
        bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
    }
    bytes
}

fn x64hash128_bytes(key: &[u8], seed: u32) -> String {
    let remainder = key.len() % 16;
    let bytes = key.len() - remainder;
    let length = X64::new(0, key.len() as u32);
    let mut h1 = X64::new(0, seed);
    let mut h2 = X64::new(0, seed);

    #[allow(clippy::chunks_exact_to_as_chunks)]
    for block in key[..bytes].chunks_exact(16) {
        let mut k1 = X64::new(
            u32::from_le_bytes([block[4], block[5], block[6], block[7]]),
            u32::from_le_bytes([block[0], block[1], block[2], block[3]]),
        );
        let mut k2 = X64::new(
            u32::from_le_bytes([block[12], block[13], block[14], block[15]]),
            u32::from_le_bytes([block[8], block[9], block[10], block[11]]),
        );

        k1.multiply(C1);
        k1.rotate_left(31);
        k1.multiply(C2);
        h1.xor(k1);
        h1.rotate_left(27);
        h1.add(h2);
        h1.multiply(M);
        h1.add(N1);

        k2.multiply(C2);
        k2.rotate_left(33);
        k2.multiply(C1);
        h2.xor(k2);
        h2.rotate_left(31);
        h2.add(h1);
        h2.multiply(M);
        h2.add(N2);
    }

    let tail = &key[bytes..];
    let mut k1 = X64::new(0, 0);
    let mut k2 = X64::new(0, 0);
    for (offset, &byte) in tail.iter().take(8).enumerate() {
        k1.xor_byte(byte, offset * 8);
    }
    for (offset, &byte) in tail.iter().skip(8).enumerate() {
        k2.xor_byte(byte, offset * 8);
    }
    if remainder > 8 {
        k2.multiply(C2);
        k2.rotate_left(33);
        k2.multiply(C1);
        h2.xor(k2);
    }
    if remainder > 0 {
        k1.multiply(C1);
        k1.rotate_left(31);
        k1.multiply(C2);
        h1.xor(k1);
    }

    h1.xor(length);
    h2.xor(length);
    h1.add(h2);
    h2.add(h1);
    h1.final_mix();
    h2.final_mix();
    h1.add(h2);
    h2.add(h1);

    let mut result = String::with_capacity(32);
    for value in [h1.high, h1.low, h2.high, h2.low] {
        push_hex(&mut result, value);
    }
    result
}

fn push_hex(output: &mut String, value: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for shift in (0..8).rev() {
        output.push(char::from(HEX[((value >> (shift * 4)) & 0xf) as usize]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_upstream_vectors() {
        assert_eq!(
            x64hash128("Hello, world, hi"),
            "9a66b4567d520770dc8eaf9a508ecf1b"
        );
        assert_eq!(
            x64hash128("Hello, world, h"),
            "4552c6409e0a7bd3b0f9eb318bb35f05"
        );
        assert_eq!(
            x64hash128("Hello, world, hi!"),
            "dce3e02d43da4d2374e84e484c566492"
        );
        assert_eq!(
            x64hash128("Hello, world, hi, Hello, world, hi"),
            "d49c261c833b671870b471c42df4dbf0"
        );
        assert_eq!(x64hash128("hello"), "cbd8a7b341bd9b025b1e906a48ae1d19");
        let non_ascii = "Hello, world, hi"
            .chars()
            .map(|character| char::from_u32(u32::from(character) + 256).unwrap())
            .collect::<String>();
        assert_eq!(x64hash128(&non_ascii), "460892d2cab76edff07f62a97e106f6b");
    }

    #[test]
    fn utf16_replaces_unpaired_surrogates() {
        assert_eq!(
            x64hash128_utf16(&[u16::from(b'a'), 0xd800, u16::from(b'b')], 0),
            x64hash128("a\u{fffd}b")
        );
    }
}
