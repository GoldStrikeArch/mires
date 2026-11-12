// Heap rows of width one: Vec<[u64; 1]> -> Box<[[u64; 1]]> -> Vec, push, Rc, and a boxed grid clone.
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u64]) -> u64 { v.iter().fold(0u64, |x, y| x * 100 + *y) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u64; 1]> = (0..4u64).map(|i| [i + 30]).collect();
    print_u64(s(v.as_flattened()));
    let bs: Box<[[u64; 1]]> = v.into_boxed_slice();
    print_u64(s(bs.as_flattened()));
    let mut v2 = bs.into_vec();
    print_u64(s(v2.as_flattened()));
    v2.push([34]);
    print_u64(s(v2.as_flattened()));
    print_u64(v2[4][0] + v2.len() as u64 * 1000);
    let rc: Rc<[[u64; 1]]> = Rc::from(v2);
    print_u64(s(rc.as_flattened()));
    let g: Box<[[u64; 1]; 2]> = Box::new(bb([[7], [8]]));
    let h = g.clone();
    print_u64(s(h.as_flattened()) + s(g.as_flattened()) * 10000);
    let mut p: Vec<[u8; 1]> = Vec::new();
    for i in 0..5u8 { p.push([i + 1]); }
    let pb = p.into_boxed_slice();
    let mut p2 = pb.into_vec();
    p2.push([9]);
    print_u64(p2.as_flattened().iter().fold(0u64, |x, y| x * 10 + *y as u64));
}
