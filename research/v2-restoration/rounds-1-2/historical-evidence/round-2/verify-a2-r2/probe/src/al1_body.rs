include!("fam_common.rs");
use alloc::alloc::{alloc, alloc_zeroed, dealloc, realloc, Layout};
#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe {
        let l = Layout::new::<[[u32; 2]; 3]>();
        let p = alloc(l) as *mut [u32; 2];
        for i in 0..3 { p.add(i).write([i as u32, 10 + i as u32]); }
        { let s = core::slice::from_raw_parts_mut(p, 3); out(1, hr(s) ^ hs(s.as_flattened())); }
        let p2 = realloc(p as *mut u8, l, 40) as *mut [u32; 2];
        p2.add(3).write([7, 8]);
        p2.add(4).write([9, 10]);
        { let s = core::slice::from_raw_parts_mut(p2, 5); out(2, hr(s) ^ hs(s.as_flattened())); }
        let q = p2 as *mut u32;
        out(3, *q.add(5) as u64);
        *q.add(4) = 44;
        out(4, (*p2.add(2))[0] as u64);
        dealloc(p2 as *mut u8, Layout::from_size_align_unchecked(40, 4));
        let z = alloc_zeroed(l) as *mut [u32; 2];
        out(5, (*z.add(2))[1] as u64);
        (*z.add(1))[0] = 6;
        { let s = core::slice::from_raw_parts(z, 3); out(6, hr(s) ^ hs(s.as_flattened())); }
        dealloc(z as *mut u8, l);
        let l8 = Layout::new::<[[u8; 4]; 3]>();
        let p = alloc(l8) as *mut [u8; 4];
        for i in 0..3 { p.add(i).write([i as u8, 10, 20, 30]); }
        { let s = core::slice::from_raw_parts_mut(p, 3); out(7, hr(s) ^ hs(s.as_flattened())); }
        let p2 = realloc(p as *mut u8, l8, 20) as *mut [u8; 4];
        p2.add(3).write([7, 8, 9, 10]);
        p2.add(4).write([9, 10, 11, 12]);
        { let s = core::slice::from_raw_parts_mut(p2, 5); out(8, hr(s) ^ hs(s.as_flattened())); }
        dealloc(p2 as *mut u8, Layout::from_size_align_unchecked(20, 1));
        let z = alloc_zeroed(l8) as *mut [u8; 4];
        out(9, (*z.add(2))[1] as u64);
        (*z.add(1))[0] = 6;
        { let s = core::slice::from_raw_parts(z, 3); out(10, hr(s) ^ hs(s.as_flattened())); }
        dealloc(z as *mut u8, l8);
    }
}
