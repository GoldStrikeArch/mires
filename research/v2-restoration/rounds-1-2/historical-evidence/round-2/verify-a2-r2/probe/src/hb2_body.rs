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
    let mut m: BTreeMap<[u8; 4], u32> = BTreeMap::new();
    for i in 0..60u32 { *m.entry(key4(i % 40)).or_insert(0) += i; m.insert([i as u8, 0, 0, 1], i); }
    out(30, m.len() as u64);
    out(31, m.iter().fold(7u64, |a, (k, v)| a.wrapping_mul(31).wrapping_add(hs(k)).wrapping_add(*v as u64)));
    out(32, m.get(&[5, 0, 0, 1]).copied().unwrap_or(999) as u64);
    out(33, m.range([3, 0, 0, 0]..[9, 0, 0, 0]).count() as u64);
    m.remove(&[5, 0, 0, 1]);
    let ks: Vec<[u8; 4]> = m.keys().copied().collect();
    out(34, hr(&ks));
    let s: BTreeSet<[u8; 32]> = (0..40u32).map(|i| digest(i % 30)).collect();
    out(35, s.len() as u64);
    out(36, s.iter().fold(7u64, |a, k| a.wrapping_mul(31).wrapping_add(hs(k))));
    out(37, s.contains(&digest(3)) as u64 + s.contains(&digest(33)) as u64 * 2);
    let mut mm: BTreeMap<[u8; 32], Vec<[u8; 4]>> = BTreeMap::new();
    for i in 0..50u32 { mm.entry(digest(i % 7)).or_default().push(key4(i)); }
    out(38, mm.values().fold(7u64, |a, v| a.wrapping_mul(31).wrapping_add(hr(v))));
    let first = mm.first_key_value().map(|(k, v)| hs(k) ^ v.len() as u64).unwrap();
    out(39, first);
    let sv: Vec<[u8; 32]> = s.into_iter().collect();
    out(40, hr(&sv));
    let mut pairs: Vec<([u8; 4], u32)> = m.into_iter().collect();
    pairs.sort_by(|a, b| b.0.cmp(&a.0));
    out(41, pairs.iter().fold(7u64, |a, (k, v)| a.wrapping_mul(31).wrapping_add(hs(k)).wrapping_add(*v as u64)));
    let mut vm: BTreeMap<u32, [u8; 4]> = BTreeMap::new();
    for i in 0..30u32 { vm.insert(i * 7 % 30, key4(i)); }
    vm.get_mut(&7).unwrap()[2] = 200;
    out(42, vm.values().fold(7u64, |a, v| a.wrapping_mul(31).wrapping_add(hs(v))));
}
