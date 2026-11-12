// A zeroed Vec<u8> whose element pointer is cast to *const [u8; 4] (the supported
// "element to array of elements" cast, CONTRACT.md cast matrix) and read.
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u8]) -> u32 { v.iter().fold(0u32, |x, y| x * 3 + *y as u32) }
#[inline(never)] fn word(p: *const u8) -> u32 { u32::from_le_bytes(unsafe { *(p as *const [u8; 4]) }) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v = bb(vec![0u8; 16]);
    print_u32(word(v.as_ptr()));
    print_usize(v.len());
    v[5] = 7;
    v[15] = 9;
    print_u32(v[5] as u32);
    print_u32(v[15] as u32);
    print_u32(s(&v));
    print_u32(word(unsafe { v.as_ptr().add(4) }));
    // Vec<u8> from TryFrom<Vec<u8>> for [u8; 4] on a truncated zero vector, then reuse via Err path
    let mut t = bb(vec![0u8; 8]);
    t.truncate(4);
    let r: Result<[u8; 4], Vec<u8>> = t.try_into();
    print_bool(r.is_ok());
    let t2 = bb(vec![0u8; 8]);
    let r2: Result<[u8; 4], Vec<u8>> = t2.try_into();
    match r2 { Ok(_) => print_u32(1), Err(v) => { print_usize(v.len()); print_u32(s(&v)); } }
}
