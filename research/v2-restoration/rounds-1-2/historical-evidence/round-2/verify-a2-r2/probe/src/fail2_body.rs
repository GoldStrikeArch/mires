// A boxed array of bytes seen as a slice of one row (safe Rust): whole-row stores through the slice.
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)]
fn store<const N: usize>(rows: &mut [[u8; N]], v: u8) {
    rows[0] = [v; N];
}
#[inline(never)]
fn store32<const N: usize>(rows: &mut [[u32; N]], v: u32) {
    rows[0] = [v; N];
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut b1: Box<[u8; 2]> = Box::new(bb([10, 11]));
    {
        let rows = core::slice::from_mut(&mut *b1);
        rows[0] = [20, 21];
        print_u32(rows[0][0] as u32);
    }
    print_u32(b1[0] as u32);
    store(core::slice::from_mut(&mut *b1), 30);
    print_u32(b1[1] as u32);
    let mut b4: Box<[u8; 4]> = Box::new(bb([1, 2, 3, 4]));
    store(core::slice::from_mut(&mut *b4), 40);
    print_u32(b4[3] as u32);
    { let r: &mut [u8; 4] = &mut *b4; *r = [50; 4]; }
    print_u32(b4[2] as u32);
    *b4 = [60; 4];
    print_u32(b4[2] as u32);
    let mut c: Box<[u32; 2]> = Box::new(bb([10, 11]));
    store32(core::slice::from_mut(&mut *c), 70);
    print_u32(c[1]);
    let mut s = bb([1u8, 2, 3]);
    store(core::slice::from_mut(&mut s), 80);
    print_u32(s[2] as u32);
    let mut v: Vec<u8> = bb(alloc::vec![1, 2, 3, 4]);
    { let a: &mut [u8; 4] = (&mut v[..]).try_into().unwrap(); store(core::slice::from_mut(a), 90); }
    print_u32(v[3] as u32);
    { let (ch, _) = v.as_chunks_mut::<2>(); store(&mut ch[1..], 91); }
    print_u32(v[3] as u32);
    let mut bs: Box<[u8]> = bb(alloc::vec![1u8, 2, 3, 4]).into_boxed_slice();
    { let a: &mut [u8; 4] = (&mut bs[..]).try_into().unwrap(); store(core::slice::from_mut(a), 92); }
    print_u32(bs[0] as u32);
    let mut ba: Box<[u8; 4]> = bs.try_into().unwrap();
    store(core::slice::from_mut(&mut *ba), 93);
    print_u32(ba[0] as u32);
}
