// Safe Rust: a one-byte heap block seen as `&[u8; 1]` (array::from_ref, first_chunk::<1>, try_into).
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn one(x: &u8) -> &[u8; 1] { core::array::from_ref(x) }
#[inline(never)] fn le(v: &[u8]) -> u8 { let a: &[u8; 1] = v[..1].try_into().unwrap(); a[0] }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let b = Box::new(bb(9u8));
    let a = one(&b);
    print_u32(a[0] as u32);
    print_u32(*b as u32);
    let mut v: Vec<u8> = Vec::with_capacity(1);
    v.push(bb(7));
    let a = one(&v[0]);
    print_u32(a[0] as u32);
    print_u32(v[0] as u32);
    print_u32(le(&v) as u32);
    v.push(8);
    print_u32(v.iter().map(|x| *x as u32).sum());
    let bx: Box<[u8; 1]> = Box::new(bb([5]));
    let c: &[u8; 1] = bx[..].first_chunk::<1>().unwrap();
    print_u32(c[0] as u32);
    print_u32(bx[0] as u32);
    print_u32(le(&bx[..]) as u32);
    let z = bb(vec![0u8; 1]);
    let a = one(&z[0]);
    print_u32(a[0] as u32 + z[0] as u32 + 1);
    print_u32(le(&z) as u32 + z.len() as u32);
    let s = bb(alloc::string::String::from("x"));
    let a = one(&s.as_bytes()[0]);
    print_u32(a[0] as u32);
    print_str(&s);
    let mut w: Vec<u8> = bb(vec![1, 2, 3, 4]);
    let a = one(&w[2]);
    print_u32(a[0] as u32);
    w[2] = 9;
    print_u32(w.iter().fold(0u32, |x, y| x * 10 + *y as u32));
    let f = bb(vec![true]);
    let a: &[bool; 1] = core::array::from_ref(&f[0]);
    print_bool(a[0] && f[0]);
}
