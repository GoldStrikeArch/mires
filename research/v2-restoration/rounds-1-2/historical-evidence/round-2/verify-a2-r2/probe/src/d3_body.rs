include!("fam_common.rs");
fn h2<T: El, const N: usize, const M: usize>(v: &[[[T; N]; M]]) -> u64 { v.iter().fold(11u64, |a, g| a.wrapping_mul(17).wrapping_add(hr(g))) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v: Vec<[[[u32; 2]; 2]; 2]> = Vec::new();
    for i in 0..3u32 { v.push([[[i, i + 1], [i + 2, i + 3]], [[i + 4, i + 5], [i + 6, i + 7]]]); }
    out(1, h2(v.as_flattened()) ^ hr(v.as_flattened().as_flattened()));
    out(2, hs(v.as_flattened().as_flattened().as_flattened()));
    v.as_flattened_mut().as_flattened_mut().as_flattened_mut()[13] = 99;
    out(3, v[1][1][0][1] as u64);
    v[2][0][1][0] = 77;
    out(4, hs(v.as_flattened().as_flattened().as_flattened()));
    v.as_flattened_mut()[3] = [[5, 5], [6, 6]];
    out(5, hs(v.as_flattened().as_flattened().as_flattened()));
    let f1: Vec<[[u32; 2]; 2]> = v.clone().into_flattened();
    out(6, h2(&f1) ^ hr(f1.as_flattened()));
    let f2: Vec<[u32; 2]> = f1.into_flattened();
    out(7, hr(&f2));
    let mut f3: Vec<u32> = f2.into_flattened();
    f3.push(1);
    out(8, hs(&f3));
    let b: Box<[[[u8; 2]; 2]; 2]> = Box::new([[[1, 2], [3, 4]], [[5, 6], [7, 8]]]);
    out(9, hs(b.as_flattened().as_flattened()));
    let mut w: Vec<[[[u8; 2]; 2]; 2]> = vec![*b; 2];
    w.push([[[9; 2]; 2]; 2]);
    w.as_flattened_mut().as_flattened_mut().as_flattened_mut()[9] = 42;
    out(10, w[1][0][0][1] as u64 + hs(w.as_flattened().as_flattened().as_flattened()));
    let g: Vec<u8> = w.into_flattened().into_flattened().into_flattened();
    out(11, hs(&g));
    let mut x: Vec<[[[u64; 1]; 1]; 1]> = vec![[[[5]]], [[[6]]]];
    x.push([[[7]]]);
    x.as_flattened_mut().as_flattened_mut().as_flattened_mut()[1] = 9;
    out(12, x[1][0][0][0] + hs(x.as_flattened().as_flattened().as_flattened()));
    let y: Vec<u64> = x.into_flattened().into_flattened().into_flattened();
    out(13, hs(&y));
}
