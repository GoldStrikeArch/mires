include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // writes through chunks of flat buffers
    let mut v: Vec<u32> = (0..11u32).map(|i| i * 3).collect();
    { let (c, r) = v.as_chunks_mut::<3>(); c[0][1] = 100; c[2] = [7, 8, 9]; r[0] = 55; c.swap(0, 1); c[1].reverse(); }
    out(1, hs(&v));
    { let (c, _) = v[1..].as_chunks_mut::<2>(); c.reverse(); c[0] = c[1]; for x in c.iter_mut() { x[0] += 1; } }
    out(2, hs(&v));
    { let (c, _) = v.as_chunks_mut::<2>(); c.sort_by(|a, b| b[0].cmp(&a[0])); }
    out(3, hs(&v));
    { let (c, _) = v.as_chunks_mut::<2>(); c.as_flattened_mut()[3] = 1000; let (x, y) = c.split_at_mut(2); core::mem::swap(&mut x[0], &mut y[1]); x[1].copy_from_slice(&y[0]); }
    out(4, hs(&v));
    { let (c, _) = v.as_chunks_mut::<5>(); c.fill([1, 2, 3, 4, 5]); c[1][4] = 6; }
    out(5, hs(&v));
    { let (_, c) = v.as_rchunks_mut::<4>(); c[0][0] = 9; c[1] = [4, 3, 2, 1]; }
    out(6, hs(&v));
    let mut st = bb([1u64, 2, 3, 4, 5, 6, 7]);
    { let (c, _) = st.as_chunks_mut::<3>(); c[1] = c[0]; c[0][2] = 1 << 40; c.rotate_left(1); }
    out(7, hs(&st));
    let mut ps: Vec<P> = (0..7u32).map(P::mk).collect();
    { let (c, _) = ps.as_chunks_mut::<3>(); c[1] = c[0]; c[0][2].a = 77; c[1][0].b = 9; c.swap(0, 1); }
    out(8, hs(&ps));
    let mut ds: Vec<D> = (0..6u32).map(D::mk).collect();
    { let (c, _) = ds.as_chunks_mut::<2>(); c[1] = [D::mk(50), D::mk(51)]; c.swap(0, 2); c[0][1] = D::mk(52); }
    out(9, hs(&ds)); out(10, drops() as u64);
    drop(ds);
    out(11, live() as i64 as u64);
    let mut bx: Box<[u32]> = (0..8u32).collect();
    { let (c, _) = bx.as_chunks_mut::<4>(); c[1][3] = 44; let t = c[0]; c[0] = c[1]; c[1] = t; }
    out(12, hs(&bx));
    for ch in v.chunks_exact_mut(2) { let a: &mut [u32; 2] = ch.try_into().unwrap(); *a = [a[1], a[0] + 1]; }
    out(13, hs(&v));
    let mut by = bb([0u8; 10]);
    { let (c, r) = by.as_chunks_mut::<4>(); c[1] = [1, 2, 3, 4]; c[0][3] = 9; r[1] = 7; c.swap(0, 1); }
    out(14, hs(&by));
}
