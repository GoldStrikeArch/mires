// Flattened rows of bytes consumed as `&[u8]`: UTF-8 decoding, the string host crossing, byte
// searches and ASCII helpers, for stack rows, boxed rows, vec!-built and push-built vectors.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[inline(never)]
fn text(t: u32, rows: &mut [[u8; 4]]) {
    print_u32(t);
    let f = rows.as_flattened();
    print_u32(f.iter().map(|b| *b as u32).sum());
    match core::str::from_utf8(f) { Ok(s) => { print_str(s); print_usize(s.len()); print_str(&s[1..5]); } Err(_) => print_u32(9999) }
    print_str(&String::from_utf8_lossy(f));
    let s2: String = f.iter().map(|b| (*b as char).to_ascii_uppercase()).collect();
    print_str(&s2);
    print_u32((f == &b"abcdefghijkl"[..]) as u32 + (f.starts_with(b"abcd") as u32) * 4 + (f.ends_with(b"ijkl") as u32) * 8 + (f.contains(&b'g') as u32) * 16);
    print_u32(f.iter().position(|b| *b == b'h').unwrap_or(99) as u32 + f.binary_search(&b'f').unwrap_or(77) as u32 * 100);
    print_u32((f.eq_ignore_ascii_case(b"ABCDEFGHIJKL") as u32) << 8 ^ (f.is_ascii() as u32) << 9);
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(f);
    out.extend_from_slice(&f[2..7]);
    out.extend(f.iter().rev());
    print_str(core::str::from_utf8(&out).unwrap());
    let b: Box<[u8]> = f.into();
    let v2 = Vec::from(f);
    print_str(core::str::from_utf8(&b).unwrap());
    print_str(&String::from_utf8(v2).unwrap());
    rows.as_flattened_mut().make_ascii_uppercase();
    rows[1].make_ascii_lowercase();
    print_str(core::str::from_utf8(rows.as_flattened()).unwrap());
    print_str(core::str::from_utf8(&rows.concat()).unwrap());
    print_str(core::str::from_utf8(&rows[1]).unwrap());
    print_str(core::str::from_utf8(&rows.as_flattened()[3..9]).unwrap());
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut g = bb([*b"abcd", *b"efgh", *b"ijkl"]);
    text(0, &mut g);
    let mut bx = Box::new(bb([*b"abcd", *b"efgh", *b"ijkl"]));
    text(1, &mut bx[..]);
    let mut m = bb(vec![*b"abcd", *b"efgh", *b"ijkl"]);
    text(2, &mut m);
    let mut v: Vec<[u8; 4]> = Vec::new();
    v.push(*b"abcd"); v.push(*b"efgh"); v.push(*b"ijkl");
    text(3, &mut v);
    let s = String::from_utf8(v.into_flattened()).unwrap_or_default();
    print_str(&s);
    let m2 = String::from_utf8(m.into_flattened()).unwrap_or_default();
    print_str(&m2);
}
