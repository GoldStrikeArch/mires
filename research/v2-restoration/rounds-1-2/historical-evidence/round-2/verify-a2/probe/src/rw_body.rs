// Writing a whole row through a `&mut [T; N]` obtained from the flattened view of rows.
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u32]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(10).wrapping_add(*y)) }
#[inline(never)] fn set(a: &mut [u32; 2]) { *a = [8, 9]; }
#[inline(never)] fn set3(a: &mut [u32; 3]) { *a = [7, 8, 9]; }
#[inline(never)]
fn go(rows: &mut [[u32; 2]]) {
    // 1. try_into at a row boundary of the flattened view
    { let a: &mut [u32; 2] = (&mut rows.as_flattened_mut()[2..4]).try_into().unwrap(); set(a); }
    print_u32(s(rows.as_flattened()));
    // 2. first_chunk_mut / last_chunk_mut of the flattened view
    { let a = rows.as_flattened_mut().first_chunk_mut::<2>().unwrap(); *a = [6, 7]; }
    print_u32(s(rows.as_flattened()));
    { let a = rows.as_flattened_mut().last_chunk_mut::<2>().unwrap(); *a = [4, 5]; }
    print_u32(s(rows.as_flattened()));
    // 3. try_into straddling two rows
    { let a: &mut [u32; 2] = (&mut rows.as_flattened_mut()[1..3]).try_into().unwrap(); set(a); }
    print_u32(s(rows.as_flattened()));
    // 4. a different width from the rows
    { let a: &mut [u32; 3] = (&mut rows.as_flattened_mut()[0..3]).try_into().unwrap(); set3(a); }
    print_u32(s(rows.as_flattened()));
    // 5. as_chunks_mut row write and split_first_chunk_mut
    { let (c, _) = rows.as_flattened_mut().as_chunks_mut::<2>(); c[1] = [1, 1]; }
    print_u32(s(rows.as_flattened()));
    { let (h, t) = rows.as_flattened_mut().split_first_chunk_mut::<2>().unwrap(); *h = [2, 2]; t[0] = 3; }
    print_u32(s(rows.as_flattened()));
    // 6. element writes through the same kind of reference (not a whole-row store)
    { let a: &mut [u32; 2] = (&mut rows.as_flattened_mut()[2..4]).try_into().unwrap(); a[0] = 5; a[1] += 1; }
    print_u32(s(rows.as_flattened()));
    // 7. a row reference straight from the rows
    set(&mut rows[0]);
    { let r: &mut [u32; 2] = rows.last_mut().unwrap(); *r = [3, 3]; }
    print_u32(s(rows.as_flattened()));
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut g = bb([[1u32, 2], [3, 4], [5, 6]]);
    go(&mut g);
    print_u32(s(&g[0]) * 10000 + s(&g[1]) * 100 + s(&g[2]));
    let mut b = Box::new(bb([[1u32, 2], [3, 4], [5, 6]]));
    go(&mut b[..]);
    let mut v = bb(vec![[1u32, 2], [3, 4], [5, 6]]);
    go(&mut v);
    let mut p: Vec<[u32; 2]> = Vec::new();
    for i in 0..3 { p.push([i * 2 + 1, i * 2 + 2]); }
    go(&mut p);
    // flat buffers: the same writes through a window that is not a row
    let mut f = bb([1u32, 2, 3, 4, 5, 6]);
    { let a: &mut [u32; 2] = (&mut f[2..4]).try_into().unwrap(); set(a); }
    print_u32(s(&f));
    { let a = f.first_chunk_mut::<2>().unwrap(); *a = [6, 7]; }
    print_u32(s(&f));
    { let a = f.last_chunk_mut::<2>().unwrap(); *a = [4, 5]; }
    print_u32(s(&f));
    { let (h, t) = f.split_first_chunk_mut::<3>().unwrap(); *h = [2, 2, 2]; t[0] = 3; }
    print_u32(s(&f));
    let mut fv = bb(vec![1u32, 2, 3, 4, 5, 6]);
    { let a: &mut [u32; 2] = (&mut fv[2..4]).try_into().unwrap(); set(a); }
    print_u32(s(&fv));
    { let a: &mut [u32; 2] = (&mut fv[4..6]).try_into().unwrap(); a[0] = 0; a.swap(0, 1); }
    print_u32(s(&fv));
}
