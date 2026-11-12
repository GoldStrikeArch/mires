include!("fam_common.rs");
use alloc::alloc::{alloc, alloc_zeroed, dealloc, realloc, Layout};
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // one block, cast twice from its byte pointer with different element types, both pointers live
    unsafe {
        let l = Layout::new::<[[u32; 2]; 3]>();
        let raw = alloc(l);
        let rows = raw as *mut [u32; 2];
        for i in 0..3 { rows.add(i).write([i as u32 + 1, 10 + i as u32]); }
        let flat = raw as *mut u32;
        out(1, *flat.add(2) as u64);
        out(2, (*rows.add(1))[1] as u64);
        *flat.add(5) = 99;
        out(3, (*rows.add(2))[1] as u64);
        (*rows.add(0))[0] = 55;
        out(4, *flat as u64);
        let rows2 = raw as *mut [u32; 2];
        out(5, (*rows2.add(2))[0] as u64);
        dealloc(raw, l);
    }
}
