// Operations that consume a flattened view as an ordinary slice: comparisons, hashing, searching,
// UTF-8 decoding and the string host crossing, bulk copies between two views, joins.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::hash::{Hash, Hasher};
use core::hint::black_box as bb;
struct Fnv(u64);
impl Hasher for Fnv {
    fn finish(&self) -> u64 { self.0 }
    fn write(&mut self, bytes: &[u8]) { for b in bytes { self.0 = (self.0 ^ *b as u64).wrapping_mul(0x100000001b3); } }
}
#[inline(never)] fn hb(v: &[u8]) -> u64 { v.iter().fold(7u64, |a, x| a.wrapping_mul(31).wrapping_add(*x as u64)) }
#[inline(never)] fn hf(v: &[u32]) -> u64 { v.iter().fold(7u64, |a, x| a.wrapping_mul(31).wrapping_add(*x as u64)) }
fn o(t: u32, v: u64) { print_u32(t); print_u64(v); }
#[inline(never)]
fn text(t: u32, rows: &mut [[u8; 4]]) {
    let f = rows.as_flattened();
    match core::str::from_utf8(f) { Ok(s) => { print_str(s); o(t, s.len() as u64); print_str(&s[1..5]); o(t + 1, s.chars().rev().map(|c| c as u64).fold(0, |a, c| a * 131 + c)); } Err(_) => o(t, 9999) }
    let owned = String::from_utf8_lossy(f);
    print_str(&owned);
    let s2: String = f.iter().map(|b| (*b as char).to_ascii_uppercase()).collect();
    print_str(&s2);
    o(t + 2, hb(f));
    o(t + 3, (f == &b"abcdefghijkl"[..]) as u64 + ((f.cmp(&b"abcdefghijkm"[..]) as i32 == -1) as u64) * 2 + (f.starts_with(b"abcd") as u64) * 4 + (f.ends_with(b"ijkl") as u64) * 8 + (f.contains(&b'g') as u64) * 16);
    o(t + 4, f.iter().position(|b| *b == b'h').unwrap_or(99) as u64 + f.binary_search(&b'f').unwrap_or(77) as u64 * 100 + *f.iter().max().unwrap() as u64 * 10000);
    o(t + 5, f.cmp(&f[1..]) as i64 as u64 ^ (f.eq_ignore_ascii_case(b"ABCDEFGHIJKL") as u64) << 8 ^ (f.is_ascii() as u64) << 9);
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(f);
    out.extend_from_slice(&f[2..7]);
    out.extend(f.iter().rev());
    print_str(core::str::from_utf8(&out).unwrap());
    let b: Box<[u8]> = f.into();
    let v2 = Vec::from(f);
    o(t + 6, hb(&b) ^ hb(&v2));
    rows.as_flattened_mut().make_ascii_uppercase();
    rows[1].make_ascii_lowercase();
    print_str(core::str::from_utf8(rows.as_flattened()).unwrap());
    let parts: Vec<&[u8]> = rows.iter().map(|r| &r[..]).collect();
    print_str(core::str::from_utf8(&parts.join(&b'-')).unwrap());
    print_str(core::str::from_utf8(&rows.join(&b"::"[..])).unwrap());
    print_str(core::str::from_utf8(&rows.concat()).unwrap());
}
#[inline(never)]
fn nums(t: u32, a: &mut [[u32; 2]], b: &mut [[u32; 2]]) {
    o(t, hf(a.as_flattened()));
    o(t + 1, (a.as_flattened() == b.as_flattened()) as u64 + (a.as_flattened().iter().lt(b.as_flattened().iter())) as u64 * 2 + (a.len() == b.len()) as u64 * 4);
    // bulk copies between two views, and from/to plain storage
    b.as_flattened_mut().copy_from_slice(a.as_flattened());
    o(t + 2, hf(b.as_flattened()) ^ (a.as_flattened() == b.as_flattened()) as u64);
    let n = a.as_flattened().len();
    b.as_flattened_mut()[1..n].copy_from_slice(&a.as_flattened()[..n - 1]);
    o(t + 3, hf(b.as_flattened()));
    b.as_flattened_mut()[..2].clone_from_slice(&a.as_flattened()[n - 2..]);
    o(t + 4, hf(b.as_flattened()));
    let mut plain = vec![0u32; n];
    plain.copy_from_slice(b.as_flattened());
    plain.reverse();
    a.as_flattened_mut().copy_from_slice(&plain);
    o(t + 5, hf(a.as_flattened()) ^ hf(&plain).rotate_left(5));
    let mut it = a.as_flattened_mut().iter_mut();
    let first = it.next().unwrap();
    let last = it.next_back().unwrap();
    core::mem::swap(first, last);
    o(t + 6, hf(a.as_flattened()));
    let (x, y) = a.as_flattened_mut().split_at_mut(n / 2);
    x.copy_from_slice(&y[..n / 2]);
    y[0] = 5;
    o(t + 7, hf(a.as_flattened()));
    o(t + 8, a.as_flattened().iter().copied().map(u64::from).sum::<u64>() + a.as_flattened().iter().rev().skip(1).step_by(2).map(|x| *x as u64).product::<u64>() % 1000003);
    let z: Vec<(u32, u32)> = a.as_flattened().iter().copied().zip(b.as_flattened().iter().copied()).collect();
    o(t + 9, z.iter().fold(0u64, |acc, (p, q)| acc.wrapping_mul(31).wrapping_add((*p as u64) << 16 ^ *q as u64)));
    o(t + 10, a.as_flattened().chunks(3).map(|c| c.len() as u64 * c[0] as u64).sum::<u64>() + a.as_flattened().rchunks(4).count() as u64 + a.as_flattened().split(|x| *x % 3 == 0).count() as u64 * 1000);
    let mut sorted = a.as_flattened().to_vec();
    sorted.sort();
    a.as_flattened_mut().sort();
    o(t + 11, (a.as_flattened() == &sorted[..]) as u64 + a.as_flattened().binary_search(&sorted[n / 2]).is_ok() as u64 * 2 + hf(a.as_flattened()));
    a.as_flattened_mut().iter_mut().for_each(|x| *x = x.wrapping_mul(2654435761));
    a.as_flattened_mut().select_nth_unstable(n / 2);
    o(t + 12, a.as_flattened()[n / 2] as u64);
    o(t + 13, a.as_flattened().iter().min().copied().unwrap() as u64 ^ a.as_flattened().first().copied().unwrap() as u64 ^ (a.as_flattened().last().copied().unwrap() as u64) << 32);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut g = bb([*b"abcd", *b"efgh", *b"ijkl"]);
    text(0, &mut g);
    let mut v: Vec<[u8; 4]> = Vec::new();
    v.push(*b"abcd"); v.push(*b"efgh"); v.push(*b"ijkl");
    text(100, &mut v);
    let mut bx = Box::new(bb([*b"abcd", *b"efgh", *b"ijkl"]));
    text(200, &mut bx[..]);
    let mut a = bb([[5u32, 9], [1, 12], [7, 3], [8, 2]]);
    let mut b = bb(vec![[5u32, 9], [1, 12], [7, 4], [8, 2]]);
    nums(1000, &mut a, &mut b);
    let mut c: Vec<[u32; 2]> = Vec::with_capacity(2);
    for i in 0..4u32 { c.push([i * 7 % 5, 11 - i]); }
    let mut d: Box<[[u32; 2]; 4]> = Box::new(bb([[1, 2], [3, 4], [5, 6], [7, 8]]));
    nums(2000, &mut c, &mut d[..]);
    nums(3000, &mut d[..], &mut a);
}
