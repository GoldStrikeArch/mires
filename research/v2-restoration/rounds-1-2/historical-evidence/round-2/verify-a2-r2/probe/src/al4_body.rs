include!("fam_common.rs");
use alloc::alloc::{alloc, alloc_zeroed, dealloc, realloc, Layout};
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // a block first used as u32s, then as rows of the same u32s (bytemuck-style regrouping)
    unsafe {
        let l = Layout::new::<[u32; 6]>();
        let raw = alloc(l);
        let flat = raw as *mut u32;
        for i in 0..6 { flat.add(i).write(i as u32 + 1); }
        let rows = raw as *mut [u32; 2];
        out(1, (*rows.add(1))[1] as u64);
        (*rows.add(2))[0] = 77;
        out(2, *flat.add(4) as u64);
        { let s = core::slice::from_raw_parts(rows, 3); out(3, hr(s) ^ hs(s.as_flattened())); }
        dealloc(raw, l);
        let raw = alloc_zeroed(l);
        let rows = raw as *mut [u32; 2];
        out(4, (*rows.add(1))[1] as u64);
        let flat = raw as *mut u32;
        out(5, *flat.add(5) as u64);
        dealloc(raw, l);
    }
}
