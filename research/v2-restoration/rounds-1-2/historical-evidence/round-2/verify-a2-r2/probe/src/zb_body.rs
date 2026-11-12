// A zeroed / fresh Vec<u8> read through a u8 -> [u8; N] cast (first_chunk, try_into, as_array).
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u8]) -> u32 { v.iter().fold(0u32, |x, y| x * 3 + *y as u32) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. first_chunk on an all-zero Vec<u8>
    let mut v = bb(vec![0u8; 16]);
    let c: &[u8; 4] = v.first_chunk::<4>().unwrap();
    print_u32(c[0] as u32 + c[3] as u32);
    print_usize(v.len());
    v[5] = 7;
    v[15] = 9;
    print_u32(v[5] as u32);
    print_u32(v[15] as u32);
    print_u32(s(&v));
    // 2. try_into on an all-zero Vec<u8> (read a little-endian word out of a zeroed buffer)
    let mut w = bb(vec![0u8; 8]);
    let x = u32::from_le_bytes(w[0..4].try_into().unwrap());
    print_u32(x);
    w[6] = 3;
    print_u32(w[6] as u32);
    print_usize(w.len());
    print_u32(s(&w));
    // 3. zeroed buffer, zero written, then word read at a later offset
    let mut z = bb(vec![0u8; 12]);
    z[..4].copy_from_slice(&0u32.to_le_bytes());
    let y = u32::from_le_bytes(z[4..8].try_into().unwrap());
    print_u32(y);
    z[9] = 5;
    print_u32(s(&z));
    print_usize(z.len());
    // 4. split_first_chunk / last_chunk
    let q = bb(vec![0u8; 6]);
    let (h, t) = q.split_first_chunk::<2>().unwrap();
    print_u32(h[1] as u32 + t.len() as u32);
    let l = q.last_chunk::<2>().unwrap();
    print_u32(l[1] as u32);
    print_usize(q.len());
    print_u32(s(&q));
    // 5. boxed zeroed slice
    let mut b: alloc::boxed::Box<[u8]> = bb(vec![0u8; 8]).into_boxed_slice();
    let f = b.first_chunk::<4>().unwrap();
    print_u32(f[2] as u32);
    b[7] = 1;
    print_u32(s(&b));
    print_usize(b.len());
}
