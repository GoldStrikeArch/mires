// Stable sorts where the ELEMENT is an array (a buffer of rows), and unstable sorts for contrast.
use alloc::vec::Vec;
use core::hint::black_box as bb;
fn lcg(s: &mut u32) -> u32 { *s = s.wrapping_mul(1664525).wrapping_add(1013904223); *s >> 8 }
#[inline(never)] fn make(n: usize, seed: u32) -> Vec<[u32; 2]> { let mut s = seed; (0..n).map(|i| [lcg(&mut s) % 50, i as u32]).collect() }
#[inline(never)] fn sum(v: &[[u32; 2]]) -> u32 { v.iter().enumerate().fold(0u32, |a, (i, x)| a.wrapping_mul(31).wrapping_add(x[0] ^ x[1].wrapping_mul(7) ^ i as u32)) }
#[inline(never)] fn make1(n: usize, seed: u32) -> Vec<u32> { let mut s = seed; (0..n).map(|_| lcg(&mut s) % 1000).collect() }
#[inline(never)] fn sum1(v: &[u32]) -> u32 { v.iter().enumerate().fold(0u32, |a, (i, x)| a.wrapping_mul(31).wrapping_add(*x ^ i as u32)) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    for &n in &[20usize, 21, 200, 5000] {
        let n = bb(n);
        let mut a = make(n, 1); a.sort_by(|x, y| x[0].cmp(&y[0]).then(y[1].cmp(&x[1]))); print_u32(sum(&a));
        let mut b = make(n, 2); b.sort_by(|x, y| y[0].cmp(&x[0])); print_u32(sum(&b));
        let mut c = make(n, 3); c.sort_by_key(|x| x[0] % 7); print_u32(sum(&c));
        let mut u = make1(n, 6); u.sort_unstable(); print_u32(sum1(&u));
        let mut w = make(n, 7); w.sort_unstable_by_key(|x| x[0]); print_u32(w.iter().map(|x| x[0]).fold(0u32, |a, x| a.wrapping_mul(31).wrapping_add(x)));
    }
    // a stack array of rows sorted in place through a slice
    let mut g = bb([[5u32, 1], [3, 2], [9, 3], [1, 4], [7, 5], [3, 6]]);
    g.sort_by_key(|r| r[0]);
    print_u32(sum(&g));
}
