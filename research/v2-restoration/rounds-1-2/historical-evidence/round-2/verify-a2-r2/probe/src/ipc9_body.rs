include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<(u32, u32)> = (0..5u32).map(|i| (i, i + 7)).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().map(|(a, b)| [a, b]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9]); out(2, hr(&f));
}
