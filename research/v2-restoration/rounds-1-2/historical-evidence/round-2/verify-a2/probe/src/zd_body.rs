// Minimal: a zeroed Vec<u8>, a word read through `*const u8 as *const [u8; 4]`, a byte write, a re-read.
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn word(p: *const u8) -> u32 { u32::from_le_bytes(unsafe { *(p as *const [u8; 4]) }) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v = bb(vec![0u8; 8]);
    print_u32(word(v.as_ptr()));
    v[1] = 5;
    print_u32(word(v.as_ptr()));
    print_u32(v[1] as u32);
    print_u32(v[0] as u32 + 1);
    print_usize(v.len());
    // fresh (unzeroed) capacity, same cast before anything is written, then pushes past capacity
    let mut u: Vec<u8> = Vec::with_capacity(8);
    let p = u.as_mut_ptr() as *mut [u8; 4];
    unsafe { p.write([1, 2, 3, 4]); u.set_len(4); }
    print_u32(u[0] as u32 * 1000 + u[1] as u32 * 100 + u[2] as u32 * 10 + u[3] as u32);
    for i in 0..20u8 { u.push(i + 5); }
    print_usize(u.len());
    print_u32(u.iter().map(|x| *x as u32).sum());
}
