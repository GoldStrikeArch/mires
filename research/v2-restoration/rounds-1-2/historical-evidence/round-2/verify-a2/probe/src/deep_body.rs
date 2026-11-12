// Nested rows three deep on the stack, in a Box and in a Vec (vec!, push, grown), with writes at
// every level observed at every other level.
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s<T: Copy + Into<u64>>(v: &[T]) -> u64 { v.iter().fold(7u64, |x, y| x.wrapping_mul(31).wrapping_add((*y).into())) }
#[inline(never)] fn s2<T: Copy + Into<u64>, const N: usize>(v: &[[T; N]]) -> u64 { v.iter().fold(5u64, |x, y| x.wrapping_mul(131).wrapping_add(s(y))) }
#[inline(never)] fn s3<T: Copy + Into<u64>, const N: usize, const M: usize>(v: &[[[T; N]; M]]) -> u64 { v.iter().fold(3u64, |x, y| x.wrapping_mul(1031).wrapping_add(s2(y))) }
fn o(t: u32, v: u64) { print_u32(t); print_u64(v); }
#[inline(never)]
fn go<T: Copy + Into<u64> + From<u8>, const N: usize, const M: usize>(t: u32, c: &mut [[[T; N]; M]]) {
    o(t, s3(c));
    o(t + 1, s2(c.as_flattened()));
    o(t + 2, s(c.as_flattened().as_flattened()));
    o(t + 3, c.as_flattened().as_flattened().len() as u64);
    let n = c.as_flattened().as_flattened().len();
    if n == 0 { return; }
    // write at the innermost level
    { let f = c.as_flattened_mut().as_flattened_mut(); f[n - 1] = T::from(200); f[0] = T::from(201); f.swap(1, n - 2); }
    o(t + 4, s3(c) ^ s2(c.as_flattened()).rotate_left(3) ^ s(c.as_flattened().as_flattened()).rotate_left(9));
    // write at the middle level
    { let m = c.as_flattened_mut(); let l = m.len(); m[l - 1] = [T::from(9); N]; m.swap(0, l - 1); m[1][0] = T::from(77); }
    o(t + 5, s3(c) ^ s2(c.as_flattened()).rotate_left(3) ^ s(c.as_flattened().as_flattened()).rotate_left(9));
    // write at the outer level
    { let l = c.len(); c[l - 1][0][0] = T::from(55); c.swap(0, l - 1); c[0] = [[T::from(3); N]; M]; }
    o(t + 6, s3(c) ^ s2(c.as_flattened()).rotate_left(3) ^ s(c.as_flattened().as_flattened()).rotate_left(9));
    // bulk moves at the innermost level
    { let f = c.as_flattened_mut().as_flattened_mut(); f.copy_within(1..n - 1, 0); f.reverse(); f[n / 2..].fill(T::from(1)); f.sort_unstable_by_key(|x| (*x).into()); }
    o(t + 7, s3(c) ^ s2(c.as_flattened()).rotate_left(3) ^ s(c.as_flattened().as_flattened()).rotate_left(9));
    // iterate rows of the middle level and chunk the innermost back into them
    let mut acc = 0u64;
    for r in c.as_flattened().iter() { acc = acc.wrapping_mul(17).wrapping_add(s(r)); }
    for ch in c.as_flattened().as_flattened().chunks_exact(N) { acc = acc.wrapping_mul(19).wrapping_add(s(ch)); }
    let (rows, rest) = c.as_flattened().as_flattened().as_chunks::<N>();
    acc ^= s2(rows) ^ rest.len() as u64;
    let (planes, rest) = c.as_flattened().as_chunks::<M>();
    acc ^= s3(planes).rotate_left(11) ^ rest.len() as u64;
    o(t + 8, acc);
    let cat: Vec<T> = c.concat().concat();
    o(t + 9, s(&cat));
    let fl: Vec<T> = c.iter().flatten().flatten().copied().collect();
    o(t + 10, s(&fl));
}
fn cube<T: Copy + From<u8>, const N: usize, const M: usize, const K: usize>(b: u8) -> [[[T; N]; M]; K] {
    core::array::from_fn(|k| core::array::from_fn(|m| core::array::from_fn(|n| T::from(b + (k * M * N + m * N + n) as u8))))
}
#[inline(never)]
fn fam<T: Copy + Into<u64> + From<u8>, const N: usize, const M: usize>(t: u32) {
    let mut st: [[[T; N]; M]; 3] = bb(cube(1));
    go(t, &mut st);
    go(t + 20, &mut st[1..]);
    let mut bx: Box<[[[T; N]; M]; 3]> = Box::new(cube(2));
    go(t + 40, &mut bx[..]);
    let mut b1: Box<[[T; N]; M]> = Box::new(cube::<T, N, M, 1>(3)[0]);
    o(t + 60, s2(&b1[..]) ^ s(b1.as_flattened()));
    b1.as_flattened_mut()[N * M - 1] = T::from(99);
    b1[0][0] = T::from(98);
    o(t + 61, s2(&b1[..]) ^ s(b1.as_flattened()));
    let mut v: Vec<[[T; N]; M]> = vec![cube::<T, N, M, 1>(4)[0]; 3];
    go(t + 80, &mut v);
    let mut p: Vec<[[T; N]; M]> = Vec::new();
    for i in 0..5u8 { p.push(cube::<T, N, M, 1>(i * 9)[0]); o(t + 100 + i as u32, s(p.as_flattened().as_flattened())); }
    go(t + 120, &mut p);
    p.truncate(2);
    p.extend_from_slice(&cube::<T, N, M, 2>(50));
    o(t + 140, s3(&p) ^ s(p.as_flattened().as_flattened()));
    let mid: Vec<[T; N]> = p.into_flattened();
    o(t + 141, s2(&mid) ^ mid.len() as u64);
    let mut inner: Vec<T> = mid.into_flattened();
    inner.push(T::from(8));
    o(t + 142, s(&inner) ^ inner.len() as u64);
    let bs: Box<[[[T; N]; M]]> = vec![cube::<T, N, M, 1>(6)[0]; 2].into_boxed_slice();
    o(t + 143, s3(&bs) ^ s(bs.as_flattened().as_flattened()));
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    fam::<u8, 2, 2>(0);
    fam::<u32, 2, 3>(1000);
    fam::<u64, 3, 2>(2000);
    fam::<u16, 1, 2>(3000);
    fam::<u64, 2, 1>(4000);
}
