// Trap cost of the flattened view per operation: 1,000 rows of 4 (4,000 elements), one operation per
// phase; the phase marker is a print, and the counting runner snapshots the trap counters at each print.
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn rows(n: usize) -> Vec<[u32; 4]> { (0..n as u32).map(|i| [i, i ^ 5, i.wrapping_mul(7) % 101, 3]).collect() }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v = rows(bb(1000));
    print_u32(0); // phase 0: built
    let s: u32 = v.as_flattened().iter().fold(0u32, |a, x| a.wrapping_add(*x));
    print_u32(s); // phase 1: iter read
    for x in v.as_flattened_mut().iter_mut() { *x = x.wrapping_add(1); }
    print_u32(v[999][3]); // phase 2: iter_mut read+write
    let f = v.as_flattened();
    let mut t = 0u32;
    for i in 0..f.len() { t = t.wrapping_add(f[i]); }
    print_u32(t); // phase 3: indexed read
    v.as_flattened_mut().fill(9);
    print_u32(v[500][2]); // phase 4: fill
    let src: Vec<u32> = (0..4000u32).map(|i| 4000 - i).collect();
    v.as_flattened_mut().copy_from_slice(&src);
    print_u32(v[1][1]); // phase 5: copy_from_slice
    v.as_flattened_mut().reverse();
    print_u32(v[0][0]); // phase 6: reverse
    v.as_flattened_mut().sort_unstable();
    print_u32(v[999][3]); // phase 7: sort_unstable (4,000 elements)
    let c: Vec<u32> = v.as_flattened().to_vec();
    print_u32(c[3999]); // phase 8: to_vec
    let d: Vec<u32> = v.concat();
    print_u32(d[17]); // phase 9: concat (no view expected)
    let e: u32 = v.iter().flatten().fold(0u32, |a, x| a.wrapping_add(*x));
    print_u32(e); // phase 10: iter().flatten() (no view expected)
    v.as_flattened_mut().copy_within(0..2000, 2000);
    print_u32(v[999][3]); // phase 11: copy_within
    let mut flat: Vec<u32> = v.into_flattened();
    print_u32(flat.iter().fold(0u32, |a, x| a.wrapping_add(*x))); // phase 12: into_flattened + iter
    for i in 0..1000u32 { flat.push(i * 3 % 17); }
    print_usize(flat.len()); // phase 13: 1,000 pushes onto the flattened Vec<u32>
    flat.sort();
    print_u32(flat[2500]); // phase 14: stable sort of the 5,000-element flattened Vec<u32>
    let mut plain: Vec<u32> = flat.clone();
    plain.sort();
    print_u32(plain[2500]); // phase 15: the same sort on a clone (plain array expected)
}
