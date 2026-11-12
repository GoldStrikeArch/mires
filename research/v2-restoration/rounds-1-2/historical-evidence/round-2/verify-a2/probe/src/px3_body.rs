// Stable sorts of a Vec of a struct with an owned string (has a destructor), 20/21/200/5,000 elements.
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box as bb;
struct Doc { name: String, id: u32, score: u32 }
fn lcg(s: &mut u32) -> u32 { *s = s.wrapping_mul(1664525).wrapping_add(1013904223); *s >> 8 }
#[inline(never)] fn make(n: usize, seed: u32) -> Vec<Doc> {
    let mut s = seed;
    (0..n).map(|i| {
        let k = lcg(&mut s) % 40;
        let mut name = String::new();
        name.push((b'a' + (k % 26) as u8) as char);
        name.push((b'a' + (k / 3 % 26) as u8) as char);
        Doc { name, id: i as u32, score: k }
    }).collect()
}
#[inline(never)] fn sum(v: &[Doc]) -> u32 { v.iter().enumerate().fold(0u32, |a, (i, x)| a.wrapping_mul(31).wrapping_add(x.id ^ x.score.wrapping_mul(7) ^ (x.name.len() as u32) ^ (x.name.as_bytes()[0] as u32) << 8 ^ i as u32)) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    for &n in &[20usize, 21, 200, 5000] {
        let n = bb(n);
        let mut a = make(n, 1); a.sort_by(|x, y| x.name.cmp(&y.name)); print_u32(sum(&a));
        let mut b = make(n, 2); b.sort_by(|x, y| y.score.cmp(&x.score).then_with(|| x.name.cmp(&y.name))); print_u32(sum(&b));
        let mut c = make(n, 3); c.sort_by_key(|x| x.score % 7); print_u32(sum(&c));
        let mut d = make(n, 4); d.sort_by_cached_key(|x| x.name.clone()); print_u32(sum(&d));
    }
}
