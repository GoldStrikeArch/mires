// Which allocation made from byte rows cannot be freed on the optimizing lane: each is made,
// read and dropped in turn. The first block uses no flattening, so main can be compared.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)] fn s(v: &[u8]) -> u32 { v.iter().fold(0u32, |x, y| x.wrapping_mul(3).wrapping_add(*y as u32)) }
#[inline(never)]
fn plain(rows: &[[u8; 4]]) {
    { let c = rows.concat(); print_u32(s(&c)); }
    print_u32(1);
    { let j = rows.join(&b"::"[..]); print_u32(s(&j)); }
    print_u32(2);
    { let parts: Vec<&[u8]> = rows.iter().map(|r| &r[..]).collect(); let j = parts.join(&b'-'); print_u32(s(&j)); }
    print_u32(3);
    { let b: Box<[u8]> = (&rows[0][..]).into(); print_u32(s(&b)); }
    print_u32(4);
    { let v = Vec::from(&rows[1][..]); print_u32(s(&v)); }
    print_u32(5);
    { let o = String::from_utf8_lossy(&rows[2]); print_usize(o.len()); }
    print_u32(6);
}
#[inline(never)]
fn flat(rows: &[[u8; 4]]) {
    let f = rows.as_flattened();
    { let b: Box<[u8]> = f.into(); print_u32(s(&b)); }
    print_u32(11);
    { let v = Vec::from(f); print_u32(s(&v)); }
    print_u32(12);
    { let v = f.to_vec(); print_u32(s(&v)); }
    print_u32(13);
    { let mut out: Vec<u8> = Vec::new(); out.extend_from_slice(f); out.extend_from_slice(&f[2..7]); out.extend(f.iter().rev()); print_u32(s(&out)); }
    print_u32(14);
    { let o = String::from_utf8_lossy(f); print_usize(o.len()); }
    print_u32(15);
    { let s2: String = f.iter().map(|b| (*b as char).to_ascii_uppercase()).collect(); print_usize(s2.len()); }
    print_u32(16);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let g = bb([*b"abcd", *b"efgh", *b"ijkl"]);
    plain(&g);
    flat(&g);
    let mut v: Vec<[u8; 4]> = Vec::new();
    v.push(*b"abcd"); v.push(*b"efgh"); v.push(*b"ijkl");
    plain(&v);
    flat(&v);
}
