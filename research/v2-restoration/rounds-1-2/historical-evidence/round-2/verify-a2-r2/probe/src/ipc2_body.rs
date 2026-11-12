include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v: Vec<[u32; 2]> = Vec::new();
    for i in 0..5u32 { v.push([i, i + 10]); }
    let mut f: Vec<u32> = v.into_iter().flatten().collect();
    out(1, hs(&f)); out(2, f.len() as u64);
    f.push(7); f.insert(0, 9); out(3, hs(&f));
}
