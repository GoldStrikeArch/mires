include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u32; 2]> = bb(vec![[1, 2], [3, 4], [5, 6], [7, 8]]);
    let mut f: Vec<u32> = v.into_iter().map(|[a, b]| a * 10 + b).collect();
    out(1, hs(&f)); out(2, f.len() as u64);
    f.push(7); out(3, hs(&f));
    let mut v: Vec<[u32; 3]> = Vec::new();
    for i in 0..6u32 { v.push([i, i + 10, i + 20]); }
    let f: Vec<u32> = v.into_iter().map(|r| r[2]).collect();
    out(4, hs(&f));
}
