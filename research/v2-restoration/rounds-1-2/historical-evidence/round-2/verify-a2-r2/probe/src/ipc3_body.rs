include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v: Vec<[u8; 4]> = Vec::new();
    for i in 0..5u8 { v.push([i, i + 10, i + 20, i + 30]); }
    let mut f: Vec<u8> = v.into_iter().flatten().collect();
    out(1, hs(&f)); out(2, f.len() as u64);
    f.push(7); f.insert(0, 9); out(3, hs(&f));
    let v: Vec<[u8; 4]> = bb(vec![[1, 2, 3, 4], [5, 6, 7, 8]]);
    let f: Vec<u8> = v.into_iter().flatten().collect();
    out(4, hs(&f));
}
