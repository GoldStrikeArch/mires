include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // nested rows flattened in place
    let v: Vec<[[u32; 2]; 2]> = (0..4u32).map(|i| [[i, i + 1], [i + 2, i + 3]]).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().flatten().collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([5, 5]); out(2, hr(&f));
    let g: Vec<u32> = f.into_iter().flatten().collect();
    out(3, hs(&g));
    let v: Vec<[[u8; 2]; 2]> = (0..4u8).map(|i| [[i, i + 1], [i + 2, i + 3]]).collect();
    let mut f: Vec<[u8; 2]> = v.into_iter().flatten().collect();
    out(4, hr(&f) ^ hs(f.as_flattened()));
    f.push([5, 5]); out(5, hr(&f));
}
