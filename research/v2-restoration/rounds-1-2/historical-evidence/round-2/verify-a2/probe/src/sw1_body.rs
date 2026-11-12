use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u32]) -> u32 { v.iter().fold(0u32, |x, y| x * 10 + y) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut g = bb([[1u32, 2, 3], [4, 5, 6]]);
    g.as_flattened_mut().swap(0, 1);
    print_u32(s(g.as_flattened()));
    g.as_flattened_mut().swap(4, 5);
    print_u32(s(g.as_flattened()));
    g.as_flattened_mut().swap(2, 3);
    print_u32(s(g.as_flattened()));
    { let f = g.as_flattened_mut(); f.swap(0, 1); f.swap(4, 5); }
    print_u32(s(g.as_flattened()));
    print_u32(s(&g[0]) * 1000 + s(&g[1]));
}
