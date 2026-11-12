include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // into_flattened consumes the owner; the result regrouped in place through the allocator API
    let v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let mut f: Vec<u32> = v.into_flattened();
    out(1, hs(&f));
    let (p, len, cap) = (f.as_mut_ptr(), f.len(), f.capacity());
    core::mem::forget(f);
    let back: Vec<[u32; 2]> = unsafe { Vec::from_raw_parts(p as *mut [u32; 2], len / 2, cap / 2) };
    out(2, hr(&back));
}
