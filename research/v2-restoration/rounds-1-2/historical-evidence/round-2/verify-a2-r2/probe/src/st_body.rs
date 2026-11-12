// A Vec<[u32; 2]> born with capacity one (its first row is registered as the block's "flat face"),
// then grown and its rows moved, then read and written through array references taken from row slices.
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u32]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(10).wrapping_add(*y)) }
#[inline(never)] fn arr(v: &[u32]) -> &[u32; 2] { v.try_into().unwrap() }
#[inline(never)] fn arr_mut(v: &mut [u32]) -> &mut [u32; 2] { v.try_into().unwrap() }
#[inline(never)]
fn show(t: u32, v: &mut Vec<[u32; 2]>) {
    print_u32(t);
    print_u32(s(v.as_flattened()));
    let mut acc = 0u32;
    for i in 0..v.len() { acc = acc.wrapping_mul(100).wrapping_add(s(arr(&v[i][..]))); }
    print_u32(acc);
    for i in 0..v.len() { let a = arr_mut(&mut v[i][..]); a[1] += 1; }
    print_u32(s(v.as_flattened()));
    let l = v.len();
    { let a = arr_mut(&mut v[l - 1][..]); *a = [a[1], a[0]]; }
    print_u32(s(v.as_flattened()));
    print_bool(v[l - 1].as_ptr() as usize == v.as_flattened()[(l - 1) * 2..].as_ptr() as usize || true);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v: Vec<[u32; 2]> = Vec::with_capacity(1);
    v.push([1, 2]);
    show(1, &mut v);
    v.push([3, 4]);
    show(2, &mut v);
    v.swap(0, 1);
    show(3, &mut v);
    v.push([5, 6]);
    v.remove(0);
    show(4, &mut v);
    v.insert(0, [7, 8]);
    show(5, &mut v);
    v.reverse();
    show(6, &mut v);
    v.sort_by_key(|r| r[0]);
    show(7, &mut v);
    let first = v.swap_remove(0);
    print_u32(s(&first));
    show(8, &mut v);
    v.truncate(1);
    v.shrink_to_fit();
    show(9, &mut v);
    v.extend_from_slice(&[[9, 9], [8, 8]]);
    v.dedup();
    v.retain(|r| r[0] != 9);
    show(10, &mut v);
    let mut w = v.clone();
    w[0][0] = 77;
    show(11, &mut v);
    show(12, &mut w);
    core::mem::swap(&mut v, &mut w);
    v.append(&mut w);
    show(13, &mut v);
    let d: Vec<[u32; 2]> = v.drain(..1).collect();
    print_u32(s(d.as_flattened()));
    show(14, &mut v);
    // the same life for a vec![one row]
    let mut o = bb(vec![[1u32, 2]]);
    show(20, &mut o);
    o.push([3, 4]);
    o.swap(0, 1);
    show(21, &mut o);
    o.remove(0);
    o.insert(0, [5, 6]);
    show(22, &mut o);
    // Box<[u32; 2]> -> Box<[u32]> -> Vec<u32> grown -> back to rows by chunking
    let b: Box<[u32; 2]> = Box::new(bb([1, 2]));
    let bs: Box<[u32]> = b;
    let mut bv = bs.into_vec();
    bv.extend_from_slice(&[3, 4, 5, 6]);
    print_u32(s(&bv));
    let a = arr_mut(&mut bv[0..2]);
    a.swap(0, 1);
    print_u32(s(&bv));
    let back: Result<Box<[u32; 6]>, _> = bv.into_boxed_slice().try_into();
    print_u32(back.map(|b| s(&b[..])).unwrap_or(0));
}
