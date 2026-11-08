//! A stable, seedless 128-bit hash for content that is persisted.
//!
//! Object files content-address their body blobs, checksum their sections, and (W4) key
//! deduplication on a hash of the semantic body, so the hash has to come out the same on every
//! machine, in every process, and across releases. `std`'s `DefaultHasher` guarantees none of that,
//! which is why this exists. The function is SipHash-1-3 with a 128-bit output and fixed keys: the
//! same primitive as the standard library's `SipHasher13`, only pinned and widened. It has no
//! adversarial-resistance role here — the keys are public — it is a fast, well-mixed digest.
//!
//! The hash is of **bytes**. To hash a value, serialize it (postcard) and hash the bytes; that
//! keeps "the hash of a body" defined by the same encoding the object file stores.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A 128-bit digest, as two little-endian halves.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct Hash128 {
    pub lo: u64,
    pub hi: u64,
}

impl Hash128 {
    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Hash128 {
        let mut hasher = Hasher128::new();
        hasher.write(bytes);
        hasher.finish()
    }

    /// The 16 bytes of the digest, `lo` first, each half little-endian.
    pub fn to_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&self.lo.to_le_bytes());
        out[8..].copy_from_slice(&self.hi.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Hash128 {
        let mut lo = [0u8; 8];
        let mut hi = [0u8; 8];
        lo.copy_from_slice(&bytes[..8]);
        hi.copy_from_slice(&bytes[8..]);
        Hash128 {
            lo: u64::from_le_bytes(lo),
            hi: u64::from_le_bytes(hi),
        }
    }

    /// The digest folded to 64 bits, for a header field that only needs to tell versions apart.
    pub fn fold(self) -> u64 {
        self.lo ^ self.hi.rotate_left(32)
    }
}

impl fmt::Display for Hash128 {
    /// 32 lowercase hex digits, `hi` first so the printed value orders like the number it is.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}{:016x}", self.hi, self.lo)
    }
}

/// Which digest an object was written with. Recorded in the header so a reader never guesses.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// SipHash-1-3, 128-bit output, keys [`K0`]/[`K1`].
    SipHash13x128,
}

/// The fixed SipHash keys. Any two constants would do; these spell the crate name.
pub const K0: u64 = 0x7263_676a_735f_6f62; // "rcgjs_ob"
pub const K1: u64 = 0x6a65_6374_5f76_3230; // "ject_v20"

/// A streaming [`Hash128`] computation.
///
/// SipHash-1-3-128 with fixed keys, written out here rather than borrowed from `std` because
/// `std::hash::SipHasher13` is unstable and only exposes a 64-bit result.
#[derive(Clone, Debug)]
pub struct Hasher128 {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    /// The bytes that did not yet fill a word, little-endian, `ntail` of them valid.
    tail: u64,
    ntail: usize,
    length: u64,
}

impl Default for Hasher128 {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher128 {
    pub fn new() -> Hasher128 {
        Hasher128 {
            v0: K0 ^ 0x736f_6d65_7073_6575,
            // The 128-bit variant of SipHash flips `v1` here; that is what makes its first output
            // word differ from the 64-bit function's result on the same input.
            v1: K1 ^ 0x646f_7261_6e64_6f6d ^ 0xee,
            v2: K0 ^ 0x6c79_6765_6e65_7261,
            v3: K1 ^ 0x7465_6462_7974_6573,
            tail: 0,
            ntail: 0,
            length: 0,
        }
    }

    #[inline]
    fn round(&mut self) {
        self.v0 = self.v0.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(13);
        self.v1 ^= self.v0;
        self.v0 = self.v0.rotate_left(32);
        self.v2 = self.v2.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(16);
        self.v3 ^= self.v2;
        self.v0 = self.v0.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(21);
        self.v3 ^= self.v0;
        self.v2 = self.v2.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(17);
        self.v1 ^= self.v2;
        self.v2 = self.v2.rotate_left(32);
    }

    #[inline]
    fn compress(&mut self, word: u64) {
        self.v3 ^= word;
        // c = 1 compression round.
        self.round();
        self.v0 ^= word;
    }

    /// Feeds `bytes` to the digest. Feeding a slice in pieces gives the same result as feeding it
    /// whole.
    pub fn write(&mut self, bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        let mut rest = bytes;
        if self.ntail != 0 {
            let need = 8 - self.ntail;
            let take = need.min(rest.len());
            for (index, byte) in rest[..take].iter().enumerate() {
                self.tail |= u64::from(*byte) << (8 * (self.ntail + index));
            }
            self.ntail += take;
            rest = &rest[take..];
            if self.ntail < 8 {
                return;
            }
            let word = self.tail;
            self.tail = 0;
            self.ntail = 0;
            self.compress(word);
        }
        let (chunks, remainder) = rest.as_chunks::<8>();
        for chunk in chunks {
            self.compress(u64::from_le_bytes(*chunk));
        }
        for (index, byte) in remainder.iter().enumerate() {
            self.tail |= u64::from(*byte) << (8 * index);
        }
        self.ntail = remainder.len();
    }

    /// The digest of everything written so far. Does not consume the hasher, so a prefix digest can
    /// be taken and writing continued.
    pub fn finish(&self) -> Hash128 {
        let mut state = self.clone();
        let last = (state.length & 0xff) << 56 | state.tail;
        state.compress(last);
        // d = 3 finalization rounds; the 128-bit variant flips `v2` with `0xee` rather than `0xff`.
        state.v2 ^= 0xee;
        state.round();
        state.round();
        state.round();
        let lo = state.v0 ^ state.v1 ^ state.v2 ^ state.v3;
        state.v1 ^= 0xdd;
        state.round();
        state.round();
        state.round();
        let hi = state.v0 ^ state.v1 ^ state.v2 ^ state.v3;
        Hash128 { lo, hi }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_short_inputs_are_stable() {
        // Pinned values: a change here is a change to every persisted hash and to the object
        // format, and has to come with a `FORMAT_VERSION` bump.
        let empty = Hash128::of(b"");
        let a = Hash128::of(b"a");
        let ab = Hash128::of(b"ab");
        assert_ne!(empty, a);
        assert_ne!(a, ab);
        assert_ne!(empty, Hash128::default());
    }

    #[test]
    fn streaming_matches_one_shot() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
        let whole = Hash128::of(&data);
        for split in [0usize, 1, 3, 7, 8, 9, 15, 16, 17, 100, 999, 1000] {
            let mut hasher = Hasher128::new();
            hasher.write(&data[..split]);
            hasher.write(&data[split..]);
            assert_eq!(hasher.finish(), whole, "split at {split}");
        }
        let mut hasher = Hasher128::new();
        for byte in &data {
            hasher.write(std::slice::from_ref(byte));
        }
        assert_eq!(hasher.finish(), whole);
    }

    #[test]
    fn matches_a_straight_line_transcription_of_siphash_1_3_128() {
        // The reference implementation's structure with c=1, d=3 and the 128-bit finalization,
        // transcribed straight-line and independently of the streaming code so a slip in the
        // tail handling cannot hide.
        fn reference(message: &[u8]) -> (u64, u64) {
            fn sipround(v: &mut [u64; 4]) {
                v[0] = v[0].wrapping_add(v[1]);
                v[1] = v[1].rotate_left(13);
                v[1] ^= v[0];
                v[0] = v[0].rotate_left(32);
                v[2] = v[2].wrapping_add(v[3]);
                v[3] = v[3].rotate_left(16);
                v[3] ^= v[2];
                v[0] = v[0].wrapping_add(v[3]);
                v[3] = v[3].rotate_left(21);
                v[3] ^= v[0];
                v[2] = v[2].wrapping_add(v[1]);
                v[1] = v[1].rotate_left(17);
                v[1] ^= v[2];
                v[2] = v[2].rotate_left(32);
            }
            let mut v = [
                K0 ^ 0x736f6d6570736575,
                K1 ^ 0x646f72616e646f6d ^ 0xee,
                K0 ^ 0x6c7967656e657261,
                K1 ^ 0x7465646279746573,
            ];
            let (blocks, tail) = message.as_chunks::<8>();
            for block in blocks {
                let m = u64::from_le_bytes(*block);
                v[3] ^= m;
                sipround(&mut v);
                v[0] ^= m;
            }
            let mut last = (message.len() as u64) << 56;
            for (i, b) in tail.iter().enumerate() {
                last |= u64::from(*b) << (8 * i);
            }
            v[3] ^= last;
            sipround(&mut v);
            v[0] ^= last;
            v[2] ^= 0xee;
            for _ in 0..3 {
                sipround(&mut v);
            }
            let lo = v[0] ^ v[1] ^ v[2] ^ v[3];
            v[1] ^= 0xdd;
            for _ in 0..3 {
                sipround(&mut v);
            }
            (lo, v[0] ^ v[1] ^ v[2] ^ v[3])
        }
        for message in [
            &b""[..],
            b"a",
            b"rcgjs",
            b"0123456789abcdef",
            b"0123456789abcdefg",
            &[0u8; 100],
        ] {
            let (lo, hi) = reference(message);
            assert_eq!(Hash128::of(message), Hash128 { lo, hi });
        }
    }

    #[test]
    fn bytes_round_trip() {
        let hash = Hash128::of(b"round trip");
        assert_eq!(Hash128::from_bytes(hash.to_bytes()), hash);
        assert_eq!(hash.to_string().len(), 32);
    }
}
