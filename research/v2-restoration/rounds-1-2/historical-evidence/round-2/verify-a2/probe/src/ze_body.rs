// Safe Rust only: zeroed byte buffers read through first_chunk / try_into, in shapes that may keep the
// Vec<u8> a heap block on the optimizing lane (its pointer is observed, it sits in a struct, it is
// passed through a generic function, it is a Vec<i8> / Vec<bool>).
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u8]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(3).wrapping_add(*y as u32)) }
#[inline(never)] fn word(v: &[u8], at: usize) -> u32 { u32::from_le_bytes(v[at..at + 4].try_into().unwrap()) }
struct Buf { data: Vec<u8>, pos: usize }
impl Buf {
    #[inline(never)] fn read(&mut self) -> u32 { let w = word(&self.data, self.pos); self.pos += 4; w }
}
#[inline(never)] fn gen<T: AsRef<[u8]>>(t: &T) -> u32 { let c = t.as_ref().first_chunk::<4>().unwrap(); c[0] as u32 + c[3] as u32 }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v = bb(vec![0u8; 16]);
    print_bool(v.as_ptr() as usize != 0);
    let c: &[u8; 4] = v.first_chunk::<4>().unwrap();
    print_u32(c[0] as u32);
    v[5] = 7;
    print_u32(word(&v, 4));
    print_u32(s(&v));
    print_usize(v.len());
    let mut b = Buf { data: vec![0u8; 12], pos: 0 };
    print_u32(b.read());
    b.data[5] = 1;
    print_u32(b.read());
    print_u32(s(&b.data));
    b.data.push(9);
    print_usize(b.data.len());
    let g = bb(vec![0u8; 8]);
    print_u32(gen(&g));
    print_u32(s(&g) + g.len() as u32);
    let mut i8s = bb(vec![0i8; 8]);
    let c = i8s.first_chunk::<4>().unwrap();
    print_i32(c[1] as i32);
    i8s[5] = -3;
    print_i32(i8s.iter().map(|x| *x as i32).sum());
    let mut bs = bb(vec![false; 8]);
    let c = bs.first_chunk::<4>().unwrap();
    print_bool(c[1]);
    bs[5] = true;
    print_u32(bs.iter().filter(|x| **x).count() as u32 + bs.len() as u32 * 10);
    let mut bx: Box<[u8]> = vec![0u8; 8].into_boxed_slice();
    print_bool(bx.as_ptr() as usize != 0);
    let c = bx.last_chunk::<4>().unwrap();
    print_u32(c[3] as u32);
    bx[1] = 2;
    print_u32(s(&bx) + bx.len() as u32);
    let mut q: Vec<u8> = Vec::with_capacity(8);
    print_usize(q.first_chunk::<4>().map(|c| c[0] as usize).unwrap_or(99));
    let (ch, rest) = q.as_chunks::<4>();
    print_usize(ch.len() + rest.len());
    for i in 0..12u8 { q.push(i + 1); }
    print_u32(s(&q) + q.len() as u32);
}
