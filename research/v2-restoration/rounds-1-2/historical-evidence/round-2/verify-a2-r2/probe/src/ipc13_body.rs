include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u32; 1]> = (0..6u32).map(|i| [i + 7]).collect();
    let mut f: Vec<u32> = v.into_iter().map(|[a]| a + 1).collect();
    out(1, hs(&f));
    f.push(5); out(2, hs(&f));
}
