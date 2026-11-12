include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u32; 4]> = (0..5u32).map(|i| [i, i + 1, i + 2, i + 3]).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().map(|r| [r[0], r[3]]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9]); out(2, hr(&f));
}
