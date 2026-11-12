// Minimal: a whole row written through `&mut [u8; 2]` taken from the flattened view at a row boundary.
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u8]) -> u32 { v.iter().fold(0u32, |x, y| x * 10 + *y as u32) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut g = bb([[1u8, 2], [3, 4], [5, 6]]);
    { let a: &mut [u8; 2] = g.as_flattened_mut().first_chunk_mut::<2>().unwrap(); *a = [6, 7]; }
    print_u32(s(g.as_flattened()));
    { let a: &mut [u8; 2] = (&mut g.as_flattened_mut()[2..4]).try_into().unwrap(); *a = [8, 9]; }
    print_u32(s(g.as_flattened()));
    print_u32(s(&g[0]) * 100 + s(&g[1]));
}
