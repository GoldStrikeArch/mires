
#![no_std]
#![no_main]
#![allow(unused)]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use alloc::borrow::{Cow, ToOwned};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq)]
struct Row { name: &'static str, weight: u32, pair: (u32, u32), arr: [u32; 3] }

const ARR: &[u32; 100] = &[1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
const SL: &[u32] = &[2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2009, 2010, 2011, 2012, 2013, 2014, 2015, 2016, 2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026, 2027, 2028, 2029, 2030, 2031, 2032, 2033, 2034, 2035, 2036, 2037, 2038, 2039, 2040, 2041, 2042, 2043, 2044, 2045, 2046, 2047, 2048, 2049, 2050, 2051, 2052, 2053, 2054, 2055, 2056, 2057, 2058, 2059, 2060, 2061, 2062, 2063, 2064, 2065, 2066, 2067, 2068, 2069, 2070, 2071, 2072, 2073, 2074, 2075, 2076, 2077, 2078, 2079, 2080, 2081, 2082, 2083, 2084, 2085, 2086, 2087, 2088, 2089, 2090, 2091, 2092, 2093, 2094, 2095, 2096, 2097, 2098, 2099, 2100];
const ROWS: &[Row] = &[
    Row { name: "a2-slice-row-0-with-a-longish-name", weight: 10, pair: (100, 200), arr: [300, 400, 500] },
    Row { name: "a2-slice-row-1-with-a-longish-name", weight: 11, pair: (101, 201), arr: [301, 401, 501] },
    Row { name: "a2-slice-row-2-with-a-longish-name", weight: 12, pair: (102, 202), arr: [302, 402, 502] },
    Row { name: "a2-slice-row-3-with-a-longish-name", weight: 13, pair: (103, 203), arr: [303, 403, 503] },
    Row { name: "a2-slice-row-4-with-a-longish-name", weight: 14, pair: (104, 204), arr: [304, 404, 504] },
    Row { name: "a2-slice-row-5-with-a-longish-name", weight: 15, pair: (105, 205), arr: [305, 405, 505] },
    Row { name: "a2-slice-row-6-with-a-longish-name", weight: 16, pair: (106, 206), arr: [306, 406, 506] },
    Row { name: "a2-slice-row-7-with-a-longish-name", weight: 17, pair: (107, 207), arr: [307, 407, 507] }
];
const ROWARR: &[Row; 8] = &[
    Row { name: "a2-array-row-0-with-a-longish-name", weight: 10, pair: (100, 200), arr: [300, 400, 500] },
    Row { name: "a2-array-row-1-with-a-longish-name", weight: 11, pair: (101, 201), arr: [301, 401, 501] },
    Row { name: "a2-array-row-2-with-a-longish-name", weight: 12, pair: (102, 202), arr: [302, 402, 502] },
    Row { name: "a2-array-row-3-with-a-longish-name", weight: 13, pair: (103, 203), arr: [303, 403, 503] },
    Row { name: "a2-array-row-4-with-a-longish-name", weight: 14, pair: (104, 204), arr: [304, 404, 504] },
    Row { name: "a2-array-row-5-with-a-longish-name", weight: 15, pair: (105, 205), arr: [305, 405, 505] },
    Row { name: "a2-array-row-6-with-a-longish-name", weight: 16, pair: (106, 206), arr: [306, 406, 506] },
    Row { name: "a2-array-row-7-with-a-longish-name", weight: 17, pair: (107, 207), arr: [307, 407, 507] }
];
const BYTES: &[u8] = &[84, 104, 101, 32, 113, 117, 105, 99, 107, 32, 98, 114, 111, 119, 110, 32, 102, 111, 120, 32, 106, 117, 109, 112, 115, 32, 111, 118, 101, 114, 32, 116, 104, 101, 32, 108, 97, 122, 121, 32, 100, 111, 103, 44, 32, 97, 110, 100, 32, 116, 104, 101, 110, 32, 107, 101, 101, 112, 115, 32, 114, 117, 110, 110, 105, 110, 103, 32, 116, 104, 114, 111, 117, 103, 104, 32, 111, 110, 101, 32, 104, 117, 110, 100, 114, 101, 100, 32, 97, 110, 100, 32, 116, 119, 101, 110, 116, 121, 32, 101, 105, 103, 104, 116, 32, 109, 111, 114, 101, 32, 98, 121, 116, 101, 115, 32, 111, 102, 32, 112, 108, 97, 105, 110, 32, 65, 83, 67, 73, 73, 32, 116, 101, 120, 116, 32, 115, 111, 32, 116, 104, 101, 32, 116, 97, 98, 108, 101, 32, 105, 115, 32, 108, 97, 114, 103, 101, 32, 101, 110, 111, 117, 103, 104, 32, 116, 111, 32, 112, 111, 111, 108, 46];
const WORDS: &[&str] = &["a2-word-alpha-long", "a2-word-beta-long", "a2-word-gamma-long", "a2-word-delta-long", "a2-word-epsilon-long", "a2-word-zeta-long", "a2-word-eta-long", "a2-word-theta-long", "a2-word-iota-long", "a2-word-kappa-long", "a2-word-lambda-long", "a2-word-mu-long", "a2-word-nu-long", "a2-word-xi-long"];

#[inline(never)] fn f_vec_u32() {
    let mut v = SL.to_vec(); v[0] = 99; v.push(5); v.sort_unstable_by(|a, b| b.cmp(a)); v.truncate(3); print_u32(v[0]); print_u32(SL[0]); print_usize(SL.len());
    let mut v2 = Vec::from(SL); v2[1] = 98; v2.clear(); print_u32(SL[1]);
    let mut v3 = Vec::from(*ARR); v3[2] = 97; v3.push(1); v3.reverse(); print_u32(ARR[2]); print_u32(ARR[99]);
    let mut b = Box::new(*ARR); b[3] = 96; print_u32(ARR[3]); let mut un = *b; un[4] = 95; print_u32(ARR[4]);
    let mut o = SL.to_owned(); o[7] = 92; o.dedup(); o.retain(|x| x % 2 == 0); print_u32(SL[7]);
    let mut c: Vec<u32> = SL.iter().copied().collect(); c[8] = 91; c.drain(..10); print_u32(SL[8]);
    let mut e = Vec::new(); e.extend_from_slice(SL); e.extend_from_slice(ARR); e[9] = 90; e[109] = 89; e.swap_remove(0); print_u32(SL[9]); print_u32(ARR[9]); print_u32(SL[0]);
    let mut cat = [SL, &ARR[..]].concat(); cat[10] = 0; cat[110] = 0; print_u32(SL[10]); print_u32(ARR[10]);
    let mut rep = SL.repeat(2); rep[11] = 0; rep[111] = 0; print_u32(SL[11]);
    let rc = Rc::<[u32]>::from(SL); print_u32(rc[12]); let mut owned = Rc::try_unwrap(Rc::new(*ARR)).ok().unwrap(); owned[12] = 0; print_u32(ARR[12]);
    let mut cow: Cow<'static, [u32]> = Cow::Borrowed(SL); cow.to_mut()[13] = 0; print_u32(SL[13]); let mut io = Cow::Borrowed(SL).into_owned(); io[14] = 0; print_u32(SL[14]);
    let mut dq = VecDeque::from(SL.to_vec()); dq[15] = 0; dq.push_front(1); dq[16] = 0; print_u32(SL[15]); print_u32(SL[14]);
    let mut leak = SL.to_vec().leak(); leak[17] = 0; print_u32(SL[17]);
    let mut ins = SL.to_vec(); ins.insert(0, 7); ins.splice(1..3, [1, 2, 3]); print_u32(SL[0]); print_u32(SL[1]); print_u32(SL[2]);
    let mut it = SL.to_vec().into_iter(); let mut first = it.next().unwrap(); first += 1; let rest: Vec<u32> = it.map(|x| x + 1).collect(); print_u32(rest[0]); print_u32(SL[1]);
    let mut bx = SL.to_vec().into_boxed_slice(); bx[18] = 0; let mut back = bx.into_vec(); back[19] = 0; print_u32(SL[18]); print_u32(SL[19]);
    let mut sorted = SL.to_vec(); sorted.sort_by(|a, b| b.cmp(a)); sorted[0] = 0; print_u32(SL[99]);
    let mut chunks: Vec<Vec<u32>> = SL.chunks(7).map(|c| c.to_vec()).collect(); chunks[0][0] = 0; chunks[1][0] = 0; print_u32(SL[0]); print_u32(SL[7]);
    let mut w: Vec<u32> = SL.windows(3).map(|w| w[0] + w[1] + w[2]).collect(); w[0] = 0; print_u32(SL[0]);
    let (l, r) = SL.split_at(50); let mut lv = l.to_vec(); let mut rv = r.to_vec(); lv[0] = 0; rv[0] = 0; print_u32(SL[0]); print_u32(SL[50]);
}
#[inline(never)] fn f_vec_rows() {
    let mut v = ROWS.to_vec(); v[0].weight = 999; v[0].arr[0] = 1; v[0].pair.0 = 2; print_u32(ROWS[0].weight); print_u32(ROWS[0].arr[0]); print_u32(ROWS[0].pair.0);
    let mut c: Vec<Row> = ROWS.iter().copied().collect(); c[1].arr[1] = 3; c[1].pair.1 = 4; print_u32(ROWS[1].arr[1]); print_u32(ROWS[1].pair.1);
    let mut cl: Vec<Row> = ROWS.iter().cloned().collect(); cl[2].arr[2] = 5; cl[2].pair.0 = 6; print_u32(ROWS[2].arr[2]); print_u32(ROWS[2].pair.0);
    let mut e = Vec::new(); e.extend_from_slice(ROWS); e.extend(ROWARR.iter()); e.extend(ROWARR.iter().copied()); e[3].arr[0] = 7; e[3].pair.0 = 7; e[11].arr[0] = 8; e[19].arr[0] = 9; e[19].pair.1 = 9;
    print_u32(ROWS[3].arr[0]); print_u32(ROWS[3].pair.0); print_u32(ROWARR[3].arr[0]); print_u32(ROWARR[3].pair.1);
    let mut cat = [ROWS, &ROWARR[..]].concat(); cat[4].arr[0] = 0; cat[4].pair.0 = 0; cat[12].arr[0] = 0; print_u32(ROWS[4].arr[0]); print_u32(ROWS[4].pair.0); print_u32(ROWARR[4].arr[0]);
    let mut rep = ROWS.repeat(2); rep[5].arr[0] = 0; rep[13].pair.0 = 0; print_u32(ROWS[5].arr[0]); print_u32(ROWS[5].pair.0);
    let mut va = Vec::from(*ROWARR); va[6].arr[0] = 0; va[6].pair.0 = 0; print_u32(ROWARR[6].arr[0]); print_u32(ROWARR[6].pair.0);
    let mut br = Box::new(ROWS[0]); br.arr[1] = 0; br.pair.1 = 0; print_u32(ROWS[0].arr[1]); print_u32(ROWS[0].pair.1);
    let mut bs = Box::<[Row]>::from(ROWS); bs[1].arr[0] = 0; bs[1].pair.0 = 0; print_u32(ROWS[1].arr[0]); print_u32(ROWS[1].pair.0);
    let rc = Rc::<[Row]>::from(ROWS); let mut one = rc[2]; one.arr[0] = 0; print_u32(ROWS[2].arr[0]);
    let mut cow: Cow<'static, [Row]> = Cow::Borrowed(ROWS); cow.to_mut()[3].arr[1] = 0; cow.to_mut()[3].pair.1 = 0; print_u32(ROWS[3].arr[1]); print_u32(ROWS[3].pair.1);
    let mut srt = ROWS.to_vec(); srt.sort_by(|a, b| b.weight.cmp(&a.weight)); srt[0].arr[0] = 0; srt[7].pair.0 = 0; print_u32(ROWS[7].arr[0]); print_u32(ROWS[0].pair.0);
    let mut m = BTreeMap::new(); for r in ROWS { m.insert(r.weight, *r); } m.get_mut(&14).unwrap().arr[0] = 0; m.get_mut(&14).unwrap().pair.0 = 0; print_u32(ROWS[4].arr[0]); print_u32(ROWS[4].pair.0);
    let mut refs: Vec<&Row> = ROWS.iter().collect(); refs.reverse(); let mut r0 = *refs[0]; r0.arr[0] = 0; print_u32(ROWS[7].arr[0]); print_str(refs[7].name);
    let mut pop = ROWS.to_vec(); let mut last = pop.pop().unwrap(); last.arr[0] = 0; last.pair.0 = 0; let mut sr = pop.swap_remove(0); sr.arr[0] = 0; let mut rm = pop.remove(0); rm.pair.0 = 0;
    print_u32(ROWS[7].arr[0]); print_u32(ROWS[7].pair.0); print_u32(ROWS[0].arr[0]); print_u32(ROWS[1].pair.0); print_u32(ROWS[6].arr[0]);
    let mut dq: VecDeque<Row> = ROWS.iter().copied().collect(); dq[0].arr[2] = 0; dq.back_mut().unwrap().pair.1 = 0; print_u32(ROWS[0].arr[2]); print_u32(ROWS[7].pair.1);
    let mut opt: Vec<Option<Row>> = ROWS.iter().map(|r| Some(*r)).collect(); if let Some(r) = opt[5].as_mut() { r.arr[2] = 0; r.pair.1 = 0; } print_u32(ROWS[5].arr[2]); print_u32(ROWS[5].pair.1);
    let mut tup: Vec<(u32, Row)> = ROWS.iter().map(|r| (r.weight, *r)).collect(); tup[6].1.arr[2] = 0; tup[6].1.pair.1 = 0; print_u32(ROWS[6].arr[2]); print_u32(ROWS[6].pair.1);
    let mut arrs: Vec<[u32; 3]> = ROWS.iter().map(|r| r.arr).collect(); arrs[0][0] = 0; let mut pairs: Vec<(u32, u32)> = ROWS.iter().map(|r| r.pair).collect(); pairs[0].0 = 0; print_u32(ROWS[0].arr[0]); print_u32(ROWS[0].pair.0);
}
#[inline(never)] fn f_bytes_strings() {
    let s = core::str::from_utf8(BYTES).unwrap(); print_usize(s.len()); print_str(&s[..9]);
    let mut owned = String::from_utf8(BYTES.to_vec()).unwrap(); owned.push_str("!!"); owned.make_ascii_uppercase(); print_str(&owned[..9]); print_u32(BYTES[0] as u32);
    let lossy = String::from_utf8_lossy(BYTES); let mut l = lossy.into_owned(); l.insert(0, 'x'); print_usize(l.len()); print_u32(BYTES[1] as u32);
    let mut bv = BYTES.to_vec(); bv[0] = b'Z'; bv.make_ascii_lowercase(); let s2 = String::from_utf8(bv).unwrap(); print_str(&s2[..5]); print_u32(BYTES[0] as u32);
    let mut up = BYTES.to_ascii_uppercase(); up[1] = b'_'; print_u32(BYTES[1] as u32); print_u32(up[2] as u32);
    let mut st = String::new(); for w in WORDS { st.push_str(w); st.push(','); } print_usize(st.len());
    let mut ws: Vec<&str> = WORDS.to_vec(); ws[0] = "changed"; ws.sort(); ws.reverse(); print_str(WORDS[0]); print_str(ws[0]);
    let joined = WORDS.join("+"); print_usize(joined.len()); let cat = WORDS.concat(); print_usize(cat.len());
    let mut strings: Vec<String> = WORDS.iter().map(|w| String::from(*w)).collect(); strings[1].push('!'); strings[2].clear(); print_str(WORDS[1]); print_str(WORDS[2]);
    let mut by = WORDS[3].as_bytes().to_vec(); by[0] = b'Q'; print_str(WORDS[3]);
    print_bool(BYTES.starts_with(b"The quick")); print_bool(BYTES.contains(&b'z')); print_usize(BYTES.iter().position(|b| *b == b',').unwrap());
    print_bool(WORDS.contains(&black_box("a2-word-mu-long"))); print_usize(WORDS.binary_search(&"a2-word-zeta-long").unwrap_or(99));
}
#[inline(never)] fn checksum() {
    let mut s = 0u32;
    for x in ARR { s = s.wrapping_mul(31).wrapping_add(*x); }
    for x in SL { s = s.wrapping_mul(31).wrapping_add(*x); }
    for r in ROWS { s = s.wrapping_mul(31).wrapping_add(r.weight + r.pair.0 * 3 + r.pair.1 * 5 + r.arr[0] * 7 + r.arr[1] * 11 + r.arr[2] * 13 + r.name.len() as u32); }
    for r in ROWARR { s = s.wrapping_mul(31).wrapping_add(r.weight + r.pair.0 * 3 + r.pair.1 * 5 + r.arr[0] * 7 + r.arr[1] * 11 + r.arr[2] * 13 + r.name.len() as u32); }
    for b in BYTES { s = s.wrapping_mul(31).wrapping_add(*b as u32); }
    for w in WORDS { s = s.wrapping_mul(31).wrapping_add(w.len() as u32 + w.as_bytes()[8] as u32); }
    print_u32(s);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    checksum();
    for round in 0..2 { f_vec_u32(); f_vec_rows(); f_bytes_strings(); checksum(); }
}
