include!("fam_common.rs");
use alloc::alloc::{alloc, alloc_zeroed, dealloc, realloc, Layout};
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // one block, rows of two widths over the same elements
    unsafe {
        let l = Layout::new::<[u32; 12]>();
        let raw = alloc(l);
        let r2 = raw as *mut [u32; 2];
        for i in 0..6 { r2.add(i).write([i as u32 * 2, i as u32 * 2 + 1]); }
        let r3 = raw as *mut [u32; 3];
        out(1, (*r3.add(1))[0] as u64);
        out(2, (*r3.add(3))[2] as u64);
        let r6 = raw as *mut [[u32; 2]; 3];
        out(3, (*r6.add(1))[2][1] as u64);
        out(4, (*r2.add(5))[1] as u64);
        dealloc(raw, l);
    }
}
