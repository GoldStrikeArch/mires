include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u32; 2]> = (0..5u32).map(|i| [i, i + 7]).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().map(|[a, b]| [b, a]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9]); out(2, hr(&f));
    let v: Vec<[u8; 4]> = (0..5u8).map(|i| [i, i + 7, 3, 4]).collect();
    let mut f: Vec<[u8; 4]> = v.into_iter().map(|[a, b, c, d]| [d, c, b, a]).collect();
    out(3, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9, 9, 9]); out(4, hr(&f));
}
