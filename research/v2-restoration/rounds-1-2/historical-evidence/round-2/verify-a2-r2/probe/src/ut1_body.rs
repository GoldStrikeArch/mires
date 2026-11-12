include!("fam_common.rs");
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
#[inline(never)]
fn digest(i: u32) -> [u8; 32] {
    let mut d = [0u8; 32];
    let mut x = i.wrapping_mul(2654435761).wrapping_add(12345);
    for b in d.iter_mut() { x ^= x << 13; x ^= x >> 17; x ^= x << 5; *b = x as u8; }
    d
}
#[inline(never)]
fn key4(i: u32) -> [u8; 4] { let x = i.wrapping_mul(2654435761); [(x >> 24) as u8, (x >> 16) as u8, (x >> 8) as u8, x as u8] }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let names: Vec<[u8; 8]> = vec![*b"alpha   ", *b"beta    ", *b"gamma\xce\xb1 ", *b"\xff\xfe      "];
    for n in &names {
        match core::str::from_utf8(n) {
            Ok(s) => { print_str(s.trim_end()); print_usize(s.chars().count()); }
            Err(e) => print_usize(e.valid_up_to()),
        }
    }
    let joined = String::from_utf8_lossy(names.as_flattened());
    print_str(&joined);
    let s = String::from_utf8(names[0].to_vec()).unwrap();
    print_str(&s);
    let mut up = names.clone();
    for n in up.iter_mut() { n.make_ascii_uppercase(); }
    print_str(core::str::from_utf8(&up[1]).unwrap());
    out(1, names[0].is_ascii() as u64 + names[2].is_ascii() as u64 * 2 + names[0].eq_ignore_ascii_case(&up[0]) as u64 * 4);
    print_str(core::str::from_utf8(&names[0][..5]).unwrap());
    let mut buf: Vec<[u8; 4]> = Vec::new();
    for ch in "h\u{e9}llo w\u{f6}rld \u{2713}\u{1F600}".chars() { let mut b = [0u8; 4]; ch.encode_utf8(&mut b); buf.push(b); }
    out(2, hr(&buf));
    let total: usize = buf.iter().map(|b| core::str::from_utf8(b).map(|s| s.trim_end_matches('\0').len()).unwrap_or(99)).sum();
    out(3, total as u64);
    let mut st = String::new();
    for b in &buf { st.push_str(core::str::from_utf8(b).unwrap().trim_end_matches('\0')); }
    print_str(&st);
    let (ch, rest) = "abcdefghij".as_bytes().as_chunks::<4>();
    out(4, hr(ch) ^ hs(rest));
    let a: [u8; 8] = "12345678".as_bytes().try_into().unwrap();
    out(5, hs(&a));
    let words: Vec<[u8; 4]> = "the quick brown fox ".as_bytes().chunks_exact(4).map(|c| c.try_into().unwrap()).collect();
    out(6, hr(&words));
    let back = String::from_utf8(words.concat()).unwrap();
    print_str(&back);
    let back2 = String::from_utf8(words.into_flattened()).unwrap();
    print_str(&back2);
    let mut bs = String::from("0123456789ab").into_bytes();
    { let (ch, _) = bs.as_chunks_mut::<3>(); ch[1] = *b"xyz"; ch[3][0] = b'!'; }
    print_str(core::str::from_utf8(&bs).unwrap());
    let rows: [[u8; 3]; 2] = [*b"abc", *b"def"];
    print_str(core::str::from_utf8(rows.as_flattened()).unwrap());
    let bx: Box<[[u8; 3]; 2]> = Box::new(rows);
    print_str(core::str::from_utf8(bx.as_flattened()).unwrap());
    let cmp = names[0].as_slice().cmp(b"alpha".as_slice()) as i32;
    print_i32(cmp);
    out(7, names.iter().filter(|n| n.starts_with(b"be") || n.ends_with(b"  ")).count() as u64);
    out(8, names.iter().map(|n| n.iter().position(|b| *b == b' ').unwrap_or(8)).sum::<usize>() as u64);
}
