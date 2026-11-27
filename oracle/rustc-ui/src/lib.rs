//! Curated `rustc/tests/ui`-style `run-pass` differential test suite for `mires`.
//!
//! Run with `cargo mires test --manifest-path oracle/rustc-ui/Cargo.toml --diff`.

/// Deterministic PRNG helper for repeatable property checks across native & JS.
pub struct SplitMix64(u64);

impl SplitMix64 {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}
