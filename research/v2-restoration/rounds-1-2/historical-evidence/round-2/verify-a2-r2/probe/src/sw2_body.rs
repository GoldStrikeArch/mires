use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u32]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(10).wrapping_add(*y)) }
#[inline(never)] fn sr<const N: usize>(v: &[[u32; N]]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(1000).wrapping_add(s(y))) }
#[inline(never)]
fn w<const N: usize>(rows: &mut [[u32; N]]) {
    let len = rows.as_flattened().len();
    { let f = rows.as_flattened_mut(); f.swap(0, len - 1); f[len / 2] = 100; }
    print_u32(sr(rows));
    rows.as_flattened_mut().reverse();
    print_u32(sr(rows));
    { let f = rows.as_flattened_mut(); f.swap(0, 1); f.swap(len - 2, len - 1); }
    print_u32(sr(rows));
    print_u32(s(rows.as_flattened()));
    { let f = rows.as_flattened_mut(); for i in 0..len / 2 { f.swap(i, len - 1 - i / 2); } }
    print_u32(sr(rows));
    rows.as_flattened_mut()[1..len - 1].fill(5);
    print_u32(sr(rows));
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut g = bb([[1u32, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]]);
    w(&mut g);
    let mut g = bb([[1u32, 2, 3], [4, 5, 6]]);
    w(&mut g);
}
