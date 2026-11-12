include!("fam_common.rs");
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
#[inline(never)]
fn digest(i: u32) -> [u8; 32] {
    let mut d = [0u8; 32];
    let mut x = i.wrapping_mul(2654435761).wrapping_add(12345);
    for b in d.iter_mut() { x ^= x << 13; x ^= x >> 17; x ^= x << 5; *b = x as u8; }
    d
}
#[inline(never)]
fn key4(i: u32) -> [u8; 4] { let x = i.wrapping_mul(2654435761); [(x >> 24) as u8, (x >> 16) as u8, (x >> 8) as u8, x as u8] }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v: Vec<[u8; 32]> = Vec::new();
    for i in 0..200u32 { v.push(digest(i % 150)); }
    out(1, hr(&v));
    out(2, (v[3] == v[153]) as u64 + (v[3] == v[4]) as u64 * 2 + (v[10] < v[11]) as u64 * 4 + (v[10].cmp(&v[11]) as i32 + 1) as u64 * 8);
    v.sort();
    out(3, hr(&v));
    v.dedup();
    out(4, hr(&v) ^ v.len() as u64);
    let k = digest(77);
    out(5, v.binary_search(&k).map(|i| i as u64).unwrap_or(9999));
    out(6, v.contains(&k) as u64 + v.contains(&[0u8; 32]) as u64 * 2);
    out(7, v.iter().position(|d| *d == k).unwrap_or(9999) as u64);
    v.sort_unstable_by(|a, b| b.cmp(a));
    out(8, hr(&v));
    v.sort_by_key(|d| d[5]);
    out(9, hr(&v));
    { let mx = v.iter().max().unwrap(); let mn = v.iter().min().unwrap(); out(10, hs(mx) ^ hs(mn).rotate_left(7)); }
    let w = v.clone();
    out(11, (w == v) as u64 + (w[..3] == v[1..4]) as u64 * 2 + (w < v) as u64 * 4);
    v.retain(|d| d[0] & 1 == 0);
    out(12, hr(&v) ^ v.len() as u64);
    v.reverse();
    v.rotate_left(3);
    v.swap(0, 5);
    out(13, hr(&v));
    let c = v.concat();
    out(14, hs(&c) ^ c.len() as u64);
    out(15, hs(v.as_flattened()));
    v[2] = [7u8; 32];
    v[3][31] = 9;
    let r: [u8; 32] = v[3];
    out(16, hs(&r) ^ hr(&v));
    for d in v.iter_mut() { d.reverse(); d[0] ^= 0xff; }
    out(17, hr(&v));
    out(18, v.windows(2).filter(|w| w[0] < w[1]).count() as u64);
    out(19, v.chunks(3).map(|c| hr(c)).fold(0, |a, b| a ^ b.rotate_left(1)));
    let (a, b) = v.split_at(v.len() / 2);
    out(20, hr(a) ^ hr(b).rotate_left(3));
    let firsts: Vec<[u8; 4]> = v.iter().map(|d| *d.first_chunk::<4>().unwrap()).collect();
    out(21, hr(&firsts));
    let lasts: Vec<[u8; 4]> = v.iter().map(|d| { let t: [u8; 4] = d[28..].try_into().unwrap(); t }).collect();
    out(22, hr(&lasts));
    let m: u32 = v.iter().map(|d| (d[0] as u32) | (d[1] as u32) << 8 | (d[2] as u32) << 16 | (d[3] as u32) << 24).fold(0, |a, b| a.wrapping_add(b));
    out(23, m as u64);
    v.as_flattened_mut().fill(1);
    v[1][1] = 2;
    out(24, hr(&v));
    out(25, v.starts_with(&[[1u8; 32]]) as u64 + v.ends_with(&[[1u8; 32]]) as u64 * 2 + w.starts_with(&w[..2]) as u64 * 4);
    let mut x = w.clone();
    x.extend_from_slice(&w[..5]);
    x.sort_unstable();
    x.dedup();
    out(26, hr(&x) ^ x.len() as u64 ^ (x == w) as u64);
    let found = w.iter().filter(|d| x.binary_search(d).is_ok()).count();
    out(27, found as u64);
    let bx: Box<[[u8; 32]]> = x.into_boxed_slice();
    out(28, hr(&bx));
    let one: Box<[u8; 32]> = Box::new(digest(5));
    out(29, hs(&one[..]) ^ (*one == digest(5)) as u64 ^ (one.as_slice() == &w[0][..]) as u64);
}
