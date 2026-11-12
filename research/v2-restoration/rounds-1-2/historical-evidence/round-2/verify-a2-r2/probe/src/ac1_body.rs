use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u32]) -> u32 { v.iter().fold(0u32, |x, y| x * 10 + y) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // as_chunks round trip of a two-row grid from its second row, then of one-row grids
    let two = bb([[1u32, 2, 3], [4, 5, 6]]);
    let (c, r) = two[1..].as_flattened().as_chunks::<3>();
    print_u32(s(&c[0]) + r.len() as u32 + c.len() as u32 * 1000);
    let g = bb([[1u32, 2, 3]]);
    let (c, r) = g.as_flattened().as_chunks::<3>();
    print_u32(s(&c[0]) + r.len() as u32 + c.len() as u32 * 1000);
    let v: Vec<[u32; 3]> = bb(vec![[4, 5, 6]]);
    let (c, r) = v.as_flattened().as_chunks::<3>();
    print_u32(s(&c[0]) + r.len() as u32 + c.len() as u32 * 1000);
}
