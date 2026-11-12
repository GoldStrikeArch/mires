include!("fam_common.rs");
use core::mem::MaybeUninit;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut b: Box<MaybeUninit<[u32; 4]>> = Box::new_uninit();
    b.write([1, 2, 3, 4]);
    let b: Box<[u32; 4]> = unsafe { b.assume_init() };
    out(1, hs(&b[..]));
    let mut b: Box<MaybeUninit<[[u32; 2]; 3]>> = Box::new_uninit();
    unsafe { b.as_mut_ptr().write([[1, 2], [3, 4], [5, 6]]); }
    let mut b = unsafe { b.assume_init() };
    b.as_flattened_mut()[3] = 9;
    out(2, hr(&b[..]) ^ hs(b.as_flattened()));
    let mut s: Box<[MaybeUninit<[u32; 2]>]> = Box::new_uninit_slice(3);
    for (i, x) in s.iter_mut().enumerate() { x.write([i as u32, 7]); }
    let mut s: Box<[[u32; 2]]> = unsafe { s.assume_init() };
    s.as_flattened_mut().reverse();
    out(3, hr(&s) ^ hs(s.as_flattened()));
    let mut v: Vec<[u32; 2]> = Vec::with_capacity(4);
    v.push([1, 2]);
    { let sp = v.spare_capacity_mut(); sp[0].write([3, 4]); sp[1].write([5, 6]); }
    unsafe { v.set_len(3); }
    out(4, hr(&v) ^ hs(v.as_flattened()));
    let mut m: MaybeUninit<[[u32; 2]; 2]> = MaybeUninit::uninit();
    unsafe {
        m.as_mut_ptr().write([[1, 2], [3, 4]]);
        (m.as_mut_ptr() as *mut u32).add(3).write(8);
        (m.as_mut_ptr() as *mut [u32; 2]).add(1).cast::<u32>().write(7);
    }
    let g = unsafe { m.assume_init() };
    out(5, hr(&g));
    let mut s: Box<[MaybeUninit<u32>]> = Box::new_uninit_slice(6);
    for (i, x) in s.iter_mut().enumerate() { x.write(i as u32 * 3); }
    let s: Box<[u32]> = unsafe { s.assume_init() };
    let (ch, _) = s.as_chunks::<2>();
    out(6, hr(ch));
}
