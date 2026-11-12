// Copying a flattened view of byte rows into a new allocation (stack rows; release-balanced).
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u8]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(3).wrapping_add(*y as u32)) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let g = bb([*b"abcd", *b"efgh", *b"ijkl"]);
    let f = g.as_flattened();
    print_u32(s(f));
    let v1 = f.to_vec();
    print_u32(s(&v1));
    print_usize(v1.len());
    let v2 = Vec::from(f);
    print_u32(s(&v2));
    let b: Box<[u8]> = f.into();
    print_u32(s(&b));
    print_str(core::str::from_utf8(&b).unwrap());
    print_str(&String::from_utf8(v2).unwrap());
    let mut e: Vec<u8> = Vec::new();
    e.extend_from_slice(&f[2..7]);
    print_u32(s(&e));
    let w = bb([[1u32, 2], [3, 4]]);
    let wb: Box<[u32]> = w.as_flattened().into();
    print_u32(wb.iter().sum());
    print_u32(Vec::from(w.as_flattened()).iter().sum());
}
