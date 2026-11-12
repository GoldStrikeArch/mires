// Hashing rows and flattened rows, separately, and UTF-8 decoding of flattened heap rows.
use alloc::vec::Vec;
use core::hash::{Hash, Hasher};
use core::hint::black_box as bb;
struct Fnv(u64);
impl Hasher for Fnv {
    fn finish(&self) -> u64 { self.0 }
    fn write(&mut self, bytes: &[u8]) { for b in bytes { self.0 = (self.0 ^ *b as u64).wrapping_mul(0x100000001b3); } }
}
fn hh<T: Hash + ?Sized>(v: &T) -> u64 { let mut h = Fnv(0xcbf29ce484222325); v.hash(&mut h); h.finish() }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let g = bb([*b"abcd", *b"efgh", *b"ijkl"]);
    print_u64(hh(&g[0]));
    print_u64(hh(&g[..]));
    print_u64(hh(&g));
    let w = bb([[1u32, 2], [3, 4]]);
    print_u64(hh(&w[..]));
    print_u64(hh(&w[1]));
    print_u64(hh(g.as_flattened()));
    print_u64(hh(w.as_flattened()));
    print_u64(hh(&b"abcdefghijkl"[..]));
    let mut v: Vec<[u8; 4]> = Vec::new();
    v.push(*b"abcd"); v.push(*b"efgh"); v.push(*b"ijkl");
    print_u64(hh(&v[..]));
    print_u64(hh(v.as_flattened()));
    print_u32(v.as_flattened().iter().map(|b| *b as u32).sum());
    match core::str::from_utf8(v.as_flattened()) { Ok(s) => { print_str(s); print_usize(s.len()); } Err(_) => print_u32(9999) }
    let s = alloc::string::String::from_utf8_lossy(v.as_flattened());
    print_str(&s);
    let owned = alloc::string::String::from_utf8(v.clone().into_flattened()).unwrap();
    print_str(&owned);
}
