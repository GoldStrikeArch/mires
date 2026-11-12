// Whole-array and element writes through `&mut [T; N]` taken from FLAT buffers (no rows at all).
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u32]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(10).wrapping_add(*y)) }
#[inline(never)] fn set(a: &mut [u32; 2]) { *a = [8, 9]; }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut f = bb([1u32, 2, 3, 4, 5, 6]);
    { let a: &mut [u32; 2] = (&mut f[2..4]).try_into().unwrap(); set(a); }
    print_u32(s(&f));
    { let a = f.first_chunk_mut::<2>().unwrap(); *a = [6, 7]; }
    print_u32(s(&f));
    { let a = f.last_chunk_mut::<2>().unwrap(); *a = [4, 5]; }
    print_u32(s(&f));
    { let (h, t) = f.split_first_chunk_mut::<3>().unwrap(); *h = [2, 2, 2]; t[0] = 3; }
    print_u32(s(&f));
    { let a: &mut [u32; 2] = (&mut f[4..6]).try_into().unwrap(); a[0] = 0; a.swap(0, 1); }
    print_u32(s(&f));
    { let a: &mut [u32; 6] = (&mut f[..]).try_into().unwrap(); *a = [1, 1, 2, 2, 3, 3]; }
    print_u32(s(&f));
    let mut fv = bb(vec![1u32, 2, 3, 4, 5, 6]);
    { let a: &mut [u32; 2] = (&mut fv[2..4]).try_into().unwrap(); set(a); }
    print_u32(s(&fv));
    { let a: &mut [u32; 2] = (&mut fv[4..6]).try_into().unwrap(); a[0] = 0; a.swap(0, 1); }
    print_u32(s(&fv));
    { let a: &mut [u32; 6] = (&mut fv[..]).try_into().unwrap(); *a = [1, 1, 2, 2, 3, 3]; }
    print_u32(s(&fv));
    fv.push(4);
    print_u32(s(&fv));
    let mut b8 = bb(vec![1u8, 2, 3, 4]);
    { let a: &mut [u8; 4] = (&mut b8[..]).try_into().unwrap(); *a = [4, 3, 2, 1]; }
    b8.push(9);
    print_u32(b8.iter().fold(0u32, |x, y| x * 10 + *y as u32));
}
