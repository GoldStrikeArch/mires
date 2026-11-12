// No rows anywhere: Box<[u8]> from an ordinary slice, then UTF-8 decoding (to compare with main).
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u8]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(3).wrapping_add(*y as u32)) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let g = bb(*b"abcdefghijkl");
    let f = &g[..];
    let b: Box<[u8]> = f.into();
    print_u32(s(&b));
    print_str(core::str::from_utf8(&b).unwrap());
    let v = Vec::from(f);
    print_str(core::str::from_utf8(&v).unwrap());
}
