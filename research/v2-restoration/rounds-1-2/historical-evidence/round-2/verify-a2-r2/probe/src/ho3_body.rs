include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // a row slice held across a hand-over of the same block
    let mut v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let rows: *mut [u32; 2] = v.as_mut_ptr();
    let f: Vec<u32> = v.into_flattened();
    out(1, hs(&f));
    out(2, unsafe { (*rows.add(1))[1] } as u64);
}
