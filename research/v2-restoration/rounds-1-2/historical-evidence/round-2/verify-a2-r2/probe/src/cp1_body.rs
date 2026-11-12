include!("fam_common.rs");
#[inline(never)]
fn cpy(t: u32, v: &mut [[u8; 4]]) {
    let src = bb([1u8, 2, 3, 4]);
    v[1].copy_from_slice(&src);
    out(t + 1, hr(v));
    let s2: Vec<u8> = bb(vec![5, 6, 7, 8, 9]);
    v[2].copy_from_slice(&s2[1..]);
    out(t + 2, hr(v));
    let mut dst = [0u8; 4];
    dst.copy_from_slice(&v[1]);
    out(t + 3, hs(&dst));
    let mut d2 = vec![0u8; 6];
    d2[1..5].copy_from_slice(&v[2]);
    out(t + 4, hs(&d2));
    v[0][1..3].copy_from_slice(&d2[2..4]);
    v[3][..2].copy_from_slice(&[21, 22]);
    out(t + 5, hr(v));
    { let (a, b) = v.split_at_mut(1); a[0].copy_from_slice(&b[0]); b[1].clone_from_slice(&a[0]); }
    out(t + 6, hr(v));
    v.as_flattened_mut()[2..10].copy_from_slice(&[7u8, 8, 9, 10, 11, 12, 13, 14]);
    out(t + 7, hr(v));
    let row3 = v[3];
    v[0] = row3;
    v[1].swap_with_slice(&mut dst);
    out(t + 8, hr(v) ^ hs(&dst));
    v.copy_within(1..3, 0);
    v[3].copy_within(1..3, 0);
    out(t + 9, hr(v));
    unsafe { core::ptr::copy_nonoverlapping(v[1].as_ptr(), d2.as_mut_ptr(), 4); core::ptr::copy_nonoverlapping(d2.as_ptr().add(2), v[2].as_mut_ptr(), 3); }
    out(t + 10, hr(v) ^ hs(&d2));
    let mut big = [0u8; 16];
    big.copy_from_slice(v.as_flattened());
    big[5] = 99;
    v.as_flattened_mut().copy_from_slice(&big);
    out(t + 11, hr(v));
    let mut tmp = [[0u8; 4]; 2];
    tmp.copy_from_slice(&v[1..3]);
    tmp[0][0] = 50;
    v[2..].copy_from_slice(&tmp);
    out(t + 12, hr(v) ^ hr(&tmp));
    let mut hv: Vec<[u8; 4]> = Vec::new();
    hv.extend_from_slice(v);
    hv[1..3].copy_from_slice(&tmp);
    v[..2].copy_from_slice(&hv[2..]);
    out(t + 13, hr(v) ^ hr(&hv));
    v[0].fill(3);
    v[1][1..].fill(4);
    core::mem::swap(&mut v[0], &mut tmp[1]);
    v.swap(1, 2);
    out(t + 14, hr(v) ^ hr(&tmp));
    let r = core::mem::replace(&mut v[3], [8; 4]);
    let tk = core::mem::take(&mut v[2]);
    out(t + 15, hr(v) ^ hs(&r) ^ hs(&tk).rotate_left(9));
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut st = bb([[0u8; 4]; 4]);
    cpy(0, &mut st);
    let mut v: Vec<[u8; 4]> = vec![[0; 4]; 3];
    v.push([9; 4]);
    cpy(100, &mut v);
    let mut b: Box<[[u8; 4]; 4]> = Box::new([[0; 4]; 4]);
    cpy(200, &mut b[..]);
    let mut p: Vec<[u8; 4]> = Vec::new();
    for i in 0..6u8 { p.push([i; 4]); }
    cpy(300, &mut p[1..5]);
    out(399, hr(&p));
    let mut bs: Box<[[u8; 4]]> = p.into_boxed_slice();
    cpy(400, &mut bs[2..]);
    let mut bytes = bb(vec![0u8; 19]);
    { let (ch, _) = bytes[2..].as_chunks_mut::<4>(); cpy(500, ch); }
    out(599, hs(&bytes));
}
