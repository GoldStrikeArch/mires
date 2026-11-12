include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // flat_map with arrays: 1 x [u16; 4] -> 2 x [u16; 2]
    let v: Vec<[u16; 4]> = (0..4u16).map(|i| [i, i + 1, i + 2, i + 3]).collect();
    let mut f: Vec<[u16; 2]> = v.into_iter().flat_map(|r| [[r[0], r[1]], [r[2], r[3]]]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([5, 5]); out(2, hr(&f));
    let v: Vec<[u8; 4]> = (0..4u8).map(|i| [i, i + 1, i + 2, i + 3]).collect();
    let mut f: Vec<[u8; 2]> = v.into_iter().flat_map(|r| [[r[0], r[1]], [r[2], r[3]]]).collect();
    out(3, hr(&f) ^ hs(f.as_flattened()));
    f.push([5, 5]); out(4, hr(&f));
}
