//! Round 2: a reinterpreting view reachable in SAFE Rust through `core`'s own `Hash for [u64]`,
//! which hands a `Hasher` the bytes of the slice. The hasher below is ordinary safe code (FNV-1a
//! over an index loop). Native Rust prints a hash, then `dropped`, `7`.
#![no_std]
#![no_main]
#![forbid(unsafe_op_in_unsafe_fn)]
use core::hash::{Hash, Hasher};

unsafe extern "C" {
    safe fn js_log_i32(x: i32);
    safe fn js_log_str(s: &str);
    safe fn probe_flag() -> i32;
}

struct Guard(i32);
impl Drop for Guard {
    fn drop(&mut self) {
        js_log_str("dropped");
        js_log_i32(self.0);
    }
}

/// FNV-1a; `ones` also counts set bits (the `ctpop` intrinsic).
struct Fnv { state: u64, ones: u32, count: bool }
impl Hasher for Fnv {
    fn finish(&self) -> u64 { self.state ^ self.ones as u64 }
    fn write(&mut self, bytes: &[u8]) {
        let mut k = 0;
        while k < bytes.len() {
            let b = bytes[k];
            self.state = (self.state ^ b as u64).wrapping_mul(0x100_0000_01b3);
            if self.count { self.ones += b.count_ones(); }
            k += 1;
        }
    }
}

#[inline(never)]
fn fingerprint(words: &[u64], count: bool) -> u64 {
    let mut h = Fnv { state: 0xcbf2_9ce4_8422_2325, ones: 0, count };
    words.hash(&mut h);
    h.finish()
}

#[inline(never)]
fn check(words: &[u64], count: bool) {
    let guard = Guard(7);
    let r = fingerprint(words, count);
    js_log_i32(r as i32);
    drop(guard);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    check(&[1u64, 2, 3], probe_flag() == 1);
}
