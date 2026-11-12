include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let f: Vec<[u32; 2]> = v.into_iter().skip(1).take(3).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    let v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let mut f: Vec<[u32; 1]> = v.into_iter().map(|r| [r[1]]).collect();
    out(2, hr(&f) ^ hs(f.as_flattened()));
    f.push([5]); out(3, hr(&f));
}
