include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u32; 2]> = (0..5u32).map(|i| [i, i + 7]).collect();
    let mut f: Vec<(u32, u32)> = v.into_iter().map(|[a, b]| (b, a)).collect();
    out(1, f.iter().fold(1u64, |h, t| h * 31 + t.0 as u64 * 7 + t.1 as u64));
    f.push((9, 9)); out(2, f.len() as u64);
}
