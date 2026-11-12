include!("fam_common.rs");
use alloc::string::String;
use core::num::{NonZeroU8, Wrapping};
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key([u8; 4]);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut v: Vec<[Wrapping<u8>; 4]> = Vec::new();
    for i in 0..5u8 { v.push([Wrapping(i), Wrapping(i + 1), Wrapping(250), Wrapping(i * 3)]); }
    for r in v.iter_mut() { r[2] += Wrapping(10); }
    out(1, v.iter().flatten().fold(7u64, |a, x| a * 31 + x.0 as u64));
    out(2, v.as_flattened().iter().fold(7u64, |a, x| a * 31 + x.0 as u64));
    v.sort_by(|a, b| b[3].cmp(&a[3]));
    v.as_flattened_mut()[5] = Wrapping(77);
    out(3, v.iter().flatten().fold(7u64, |a, x| a * 31 + x.0 as u64));
    let f: Vec<Wrapping<u8>> = v.into_flattened();
    out(4, f.iter().fold(7u64, |a, x| a * 31 + x.0 as u64));
    let mut k: Vec<Key> = (0..6u8).map(|i| Key([i ^ 3, i, 9, 1])).collect();
    k.sort();
    k.dedup();
    k[1].0[2] = 5;
    out(5, k.iter().fold(7u64, |a, x| a * 31 + hs(&x.0)));
    out(6, k.binary_search(&Key([3, 0, 9, 1])).map(|i| i as u64).unwrap_or(99));
    let mut nz: Vec<[NonZeroU8; 2]> = (1..6u8).map(|i| [NonZeroU8::new(i).unwrap(), NonZeroU8::new(i * 2).unwrap()]).collect();
    nz.reverse();
    nz.as_flattened_mut()[3] = NonZeroU8::new(200).unwrap();
    out(7, nz.iter().flatten().fold(7u64, |a, x| a * 31 + x.get() as u64));
    let mut on: Vec<[Option<NonZeroU8>; 3]> = (0..5u8).map(|i| [NonZeroU8::new(i), None, NonZeroU8::new(i + 1)]).collect();
    on.as_flattened_mut()[4] = NonZeroU8::new(9);
    out(8, on.iter().flatten().fold(7u64, |a, x| a * 31 + x.map(|n| n.get() as u64).unwrap_or(1000)));
    let mut ib: Vec<[i8; 4]> = (0..5i8).map(|i| [i, -i, i * 20, -128]).collect();
    ib.sort();
    ib.as_flattened_mut().reverse();
    out(9, hr(&ib));
    let ub: Vec<[u8; 4]> = ib.iter().map(|r| [r[0] as u8, r[1] as u8, r[2] as u8, r[3] as u8]).collect();
    out(10, hr(&ub));
    let mut bo: Vec<[bool; 3]> = (0..5u8).map(|i| [i & 1 == 0, i & 2 == 0, i > 2]).collect();
    bo.sort();
    bo.dedup();
    out(11, hr(&bo) ^ bo.len() as u64);
    let mut ord: Vec<[core::cmp::Ordering; 2]> = vec![[core::cmp::Ordering::Less, core::cmp::Ordering::Greater]; 3];
    ord.as_flattened_mut()[2] = core::cmp::Ordering::Equal;
    out(12, ord.iter().flatten().fold(7u64, |a, x| a * 31 + (*x as i8 + 1) as u64));
    let mut ch: Vec<[u8; 1]> = "hello".bytes().map(|b| [b]).collect();
    ch.sort();
    ch.dedup();
    out(13, hr(&ch) ^ hs(ch.as_flattened()));
    let s = String::from_utf8(ch.into_flattened()).unwrap();
    print_str(&s);
}
