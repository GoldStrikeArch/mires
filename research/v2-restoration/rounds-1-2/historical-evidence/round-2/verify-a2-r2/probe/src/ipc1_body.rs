include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u32; 2]> = bb(vec![[1, 2], [3, 4], [5, 6]]);
    let mut f: Vec<u32> = v.into_iter().flatten().collect();
    out(1, hs(&f)); out(2, f.len() as u64);
    f.push(7); f.insert(0, 9); out(3, hs(&f));
}
