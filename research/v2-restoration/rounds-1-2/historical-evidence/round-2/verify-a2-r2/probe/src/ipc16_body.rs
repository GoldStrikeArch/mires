include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // into_flattened then in-place collect back over the handed-over block
    let v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let f: Vec<u32> = v.into_flattened();
    out(1, hs(&f));
    let g: Vec<u32> = f.into_iter().map(|x| x * 2).collect();
    out(2, hs(&g));
    let h: Vec<[u32; 1]> = g.into_iter().map(|x| [x]).collect();
    out(3, hr(&h));
}
