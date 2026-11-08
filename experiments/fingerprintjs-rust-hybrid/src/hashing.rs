use alloc::string::String;

use mires_js::JsValue;

use crate::kernel_murmur;

/// Computes FingerprintJS's MurmurHash3 x64 128-bit hash with seed zero.
pub fn x64hash128(input: &str) -> String {
    x64hash128_seed(input, 0)
}

/// Computes FingerprintJS's MurmurHash3 x64 128-bit hash with an explicit seed.
pub fn x64hash128_seed(input: &str, seed: u32) -> String {
    let hash: &str = kernel_murmur(JsValue::erase(input), f64::from(seed)).read();
    String::from(hash)
}

/// Hashes an arbitrary host string the same way the public `murmurX64Hash128` export does.
pub fn x64hash128_host(input: JsValue, seed: JsValue) -> JsValue {
    let seed = if seed.is_undefined() {
        0.0
    } else {
        seed.read::<f64>()
    };
    kernel_murmur(input, seed)
}
