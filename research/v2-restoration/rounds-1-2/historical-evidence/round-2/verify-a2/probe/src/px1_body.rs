// Stable sorts of an ordinary Vec<u32> at 20, 21, 200 and 5,000 elements.
use alloc::vec::Vec;
use core::hint::black_box as bb;
fn lcg(s: &mut u32) -> u32 { *s = s.wrapping_mul(1664525).wrapping_add(1013904223); *s >> 8 }
#[inline(never)] fn make(n: usize, seed: u32) -> Vec<u32> { let mut s = seed; (0..n).map(|_| lcg(&mut s) % 1000).collect() }
#[inline(never)] fn sum(v: &[u32]) -> u32 { v.iter().enumerate().fold(0u32, |a, (i, x)| a.wrapping_mul(31).wrapping_add(*x ^ i as u32)) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    for &n in &[20usize, 21, 200, 5000] {
        let n = bb(n);
        let mut a = make(n, 1); a.sort(); print_u32(sum(&a));
        let mut b = make(n, 2); b.sort_by(|x, y| y.cmp(x)); print_u32(sum(&b));
        let mut c = make(n, 3); c.sort_by_key(|x| *x % 97); print_u32(sum(&c));
        let mut d = make(n, 4); d.sort_by_cached_key(|x| *x % 89); print_u32(sum(&d));
        let mut e = make(n, 5); e[..n / 2].sort(); e[n / 2..].sort_by(|x, y| y.cmp(x)); print_u32(sum(&e));
    }
}
