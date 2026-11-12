// Row addresses with no flattening at all (so main can be compared): a row reached through a
// reference vs the same row reached as a place.
use alloc::vec;
use core::hint::black_box as bb;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let g = bb([[1u32, 2, 3], [4, 5, 6], [7, 8, 9]]);
    let base = g.as_ptr() as usize;
    print_bool(g[1].as_ptr() as usize == base + 12);
    print_bool(&g[2] as *const [u32; 3] as usize == base + 24);
    print_bool(&g[1][1] as *const u32 as usize == base + 16);
    print_bool(unsafe { g.as_ptr().add(1) } as usize == base + 12);
    let mut ok = true;
    for (i, r) in g.iter().enumerate() { ok &= r.as_ptr() as usize == base + i * 12; }
    print_bool(ok);
    let v = bb(vec![[1u32, 2, 3], [4, 5, 6]]);
    let vb = v.as_ptr() as usize;
    print_bool(v[1].as_ptr() as usize == vb + 12);
    print_bool(&v[1][2] as *const u32 as usize == vb + 20);
    print_bool(core::ptr::eq(&v[1][0], v[1].as_ptr()));
}
