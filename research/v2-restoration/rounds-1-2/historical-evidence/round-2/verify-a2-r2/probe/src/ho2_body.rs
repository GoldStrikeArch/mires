include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // the flattened view borrowed, its bytes round-tripped, then the rows read again
    let mut v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    {
        let f = v.as_flattened_mut();
        let p = f.as_mut_ptr() as *mut u8;
        let q = p as *mut u32;
        unsafe { *q.add(3) = 99; }
    }
    out(1, v[1][1] as u64);
    out(2, hr(&v));
}
