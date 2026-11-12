include!("fam_common.rs");
use core::mem::MaybeUninit;
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut b: Box<MaybeUninit<[u8; 4]>> = Box::new_uninit();
    b.write([1, 2, 3, 4]);
    let b: Box<[u8; 4]> = unsafe { b.assume_init() };
    out(1, hs(&b[..]));
    let mut b: Box<MaybeUninit<[[u8; 2]; 3]>> = Box::new_uninit();
    unsafe { b.as_mut_ptr().write([[1, 2], [3, 4], [5, 6]]); }
    let mut b = unsafe { b.assume_init() };
    b.as_flattened_mut()[3] = 9;
    out(2, hr(&b[..]) ^ hs(b.as_flattened()));
    let mut s: Box<[MaybeUninit<[u8; 2]>]> = Box::new_uninit_slice(3);
    for (i, x) in s.iter_mut().enumerate() { x.write([i as u8, 7]); }
    let mut s: Box<[[u8; 2]]> = unsafe { s.assume_init() };
    s.as_flattened_mut().reverse();
    out(3, hr(&s) ^ hs(s.as_flattened()));
    let mut v: Vec<[u8; 2]> = Vec::with_capacity(4);
    v.push([1, 2]);
    { let sp = v.spare_capacity_mut(); sp[0].write([3, 4]); sp[1].write([5, 6]); }
    unsafe { v.set_len(3); }
    out(4, hr(&v) ^ hs(v.as_flattened()));
    let mut m: MaybeUninit<[[u8; 2]; 2]> = MaybeUninit::uninit();
    unsafe {
        m.as_mut_ptr().write([[1, 2], [3, 4]]);
        (m.as_mut_ptr() as *mut u8).add(3).write(8);
        (m.as_mut_ptr() as *mut [u8; 2]).add(1).cast::<u8>().write(7);
    }
    let g = unsafe { m.assume_init() };
    out(5, hr(&g));
    let mut a: [MaybeUninit<[u8; 4]>; 3] = [const { MaybeUninit::uninit() }; 3];
    for (i, x) in a.iter_mut().enumerate() { x.write([i as u8 + 1; 4]); }
    let a: [[u8; 4]; 3] = unsafe { (&a as *const [MaybeUninit<[u8; 4]>; 3] as *const [[u8; 4]; 3]).read() };
    out(6, hr(&a) ^ hs(a.as_flattened()));
    let mut vm: Vec<MaybeUninit<[u8; 4]>> = Vec::with_capacity(2);
    vm.push(MaybeUninit::new([1, 2, 3, 4]));
    vm.push(MaybeUninit::new([5, 6, 7, 8]));
    out(7, vm.iter().fold(1u64, |h, m| h * 31 + hs(unsafe { m.assume_init_ref() })));
}
