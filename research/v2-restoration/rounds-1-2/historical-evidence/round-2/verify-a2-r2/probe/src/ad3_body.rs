include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let g = bb([[1u32, 2, 3], [4, 5, 6], [7, 8, 9]]);
    let base = g.as_ptr() as usize;
    print_bool(g[1].as_ptr() as usize == base + 12);
    print_bool(&g[2] as *const [u32; 3] as usize == base + 24);
    print_bool(&g[1][1] as *const u32 as usize == base + 16);
    print_u32(100);
    let v = bb(vec![[1u32, 2, 3], [4, 5, 6]]);
    let vb = v.as_ptr() as usize;
    print_bool(v[1].as_ptr() as usize == vb + 12);
    print_bool(&v[1] as *const [u32; 3] as usize == vb + 12);
    print_bool(&v[1][2] as *const u32 as usize == vb + 20);
    print_bool(core::ptr::eq(&v[1][0], v[1].as_ptr()));
    print_u32(200);
    let b: Box<[[u32; 3]; 2]> = Box::new(bb([[1, 2, 3], [4, 5, 6]]));
    let bb0 = b.as_ptr() as usize;
    print_bool(b[1].as_ptr() as usize == bb0 + 12);
    print_bool(&b[1][2] as *const u32 as usize == bb0 + 20);
    print_u32(300);
    let mut p: Vec<[u32; 3]> = Vec::new();
    for i in 0..4 { p.push([i, i, i]); }
    let pb = p.as_ptr() as usize;
    print_bool(p[3].as_ptr() as usize == pb + 36);
    print_bool(p.iter().enumerate().all(|(i, r)| r.as_ptr() as usize == pb + i * 12));
    print_u32(400);
    let v8 = bb(vec![[1u8, 2, 3], [4, 5, 6]]);
    let v8b = v8.as_ptr() as usize;
    print_bool(v8[1].as_ptr() as usize == v8b + 3);
    print_bool(&v8[1] as *const [u8; 3] as usize == v8b + 3);
    print_bool(&v8[1][2] as *const u8 as usize == v8b + 5);
    print_bool(v8.iter().enumerate().all(|(i, r)| r.as_ptr() as usize == v8b + i * 3));
    print_u32(500);
    let g8 = bb([[1u8, 2, 3], [4, 5, 6]]);
    let g8b = g8.as_ptr() as usize;
    print_bool(g8[1].as_ptr() as usize == g8b + 3);
    print_bool(&g8[1][2] as *const u8 as usize == g8b + 5);
}
