include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<[u16; 2]> = (0..5u16).map(|i| [i, i + 7]).collect();
    let f: Vec<u16> = v.into_iter().map(|r| r[1]).collect();
    out(1, hs(&f));
    let v: Vec<[f64; 2]> = (0..5u32).map(|i| [i as f64, 0.5]).collect();
    let f: Vec<f64> = v.into_iter().flatten().collect();
    out(2, hs(&f));
    let v: Vec<[u64; 2]> = (0..5u64).map(|i| [i << 40, i]).collect();
    let f: Vec<u64> = v.into_iter().flatten().collect();
    out(3, hs(&f));
}
