
#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;

#[derive(Clone, Copy, PartialEq)]
struct Row { name: &'static str, weight: u32, pair: (u32, u32), arr: [u32; 3] }

#[derive(Clone, Copy)]
enum Shape { Dot, Rect { w: u32, h: u32 }, Poly([u32; 6]), Named(&'static str, [u32; 4]) }

const ARR: &[u32; 100] = &[1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
const SL: &[u32] = &[2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2009, 2010, 2011, 2012, 2013, 2014, 2015, 2016, 2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026, 2027, 2028, 2029, 2030, 2031, 2032, 2033, 2034, 2035, 2036, 2037, 2038, 2039, 2040, 2041, 2042, 2043, 2044, 2045, 2046, 2047, 2048, 2049, 2050, 2051, 2052, 2053, 2054, 2055, 2056, 2057, 2058, 2059, 2060, 2061, 2062, 2063, 2064, 2065, 2066, 2067, 2068, 2069, 2070, 2071, 2072, 2073, 2074, 2075, 2076, 2077, 2078, 2079, 2080, 2081, 2082, 2083, 2084, 2085, 2086, 2087, 2088, 2089, 2090, 2091, 2092, 2093, 2094, 2095, 2096, 2097, 2098, 2099, 2100];
const ROWS: &[Row] = &[
    Row { name: "a1-slice-row-0-with-a-longish-name", weight: 10, pair: (100, 200), arr: [300, 400, 500] },
    Row { name: "a1-slice-row-1-with-a-longish-name", weight: 11, pair: (101, 201), arr: [301, 401, 501] },
    Row { name: "a1-slice-row-2-with-a-longish-name", weight: 12, pair: (102, 202), arr: [302, 402, 502] },
    Row { name: "a1-slice-row-3-with-a-longish-name", weight: 13, pair: (103, 203), arr: [303, 403, 503] },
    Row { name: "a1-slice-row-4-with-a-longish-name", weight: 14, pair: (104, 204), arr: [304, 404, 504] },
    Row { name: "a1-slice-row-5-with-a-longish-name", weight: 15, pair: (105, 205), arr: [305, 405, 505] },
    Row { name: "a1-slice-row-6-with-a-longish-name", weight: 16, pair: (106, 206), arr: [306, 406, 506] },
    Row { name: "a1-slice-row-7-with-a-longish-name", weight: 17, pair: (107, 207), arr: [307, 407, 507] }
];
const ROWARR: &[Row; 8] = &[
    Row { name: "a1-array-row-0-with-a-longish-name", weight: 10, pair: (100, 200), arr: [300, 400, 500] },
    Row { name: "a1-array-row-1-with-a-longish-name", weight: 11, pair: (101, 201), arr: [301, 401, 501] },
    Row { name: "a1-array-row-2-with-a-longish-name", weight: 12, pair: (102, 202), arr: [302, 402, 502] },
    Row { name: "a1-array-row-3-with-a-longish-name", weight: 13, pair: (103, 203), arr: [303, 403, 503] },
    Row { name: "a1-array-row-4-with-a-longish-name", weight: 14, pair: (104, 204), arr: [304, 404, 504] },
    Row { name: "a1-array-row-5-with-a-longish-name", weight: 15, pair: (105, 205), arr: [305, 405, 505] },
    Row { name: "a1-array-row-6-with-a-longish-name", weight: 16, pair: (106, 206), arr: [306, 406, 506] },
    Row { name: "a1-array-row-7-with-a-longish-name", weight: 17, pair: (107, 207), arr: [307, 407, 507] }
];
const TUP: &(u32, [u32; 80], Row) = &(5, [4001, 4002, 4003, 4004, 4005, 4006, 4007, 4008, 4009, 4010, 4011, 4012, 4013, 4014, 4015, 4016, 4017, 4018, 4019, 4020, 4021, 4022, 4023, 4024, 4025, 4026, 4027, 4028, 4029, 4030, 4031, 4032, 4033, 4034, 4035, 4036, 4037, 4038, 4039, 4040, 4041, 4042, 4043, 4044, 4045, 4046, 4047, 4048, 4049, 4050, 4051, 4052, 4053, 4054, 4055, 4056, 4057, 4058, 4059, 4060, 4061, 4062, 4063, 4064, 4065, 4066, 4067, 4068, 4069, 4070, 4071, 4072, 4073, 4074, 4075, 4076, 4077, 4078, 4079, 4080], Row { name: "a1-tuple-row", weight: 1, pair: (2, 3), arr: [4, 5, 6] });
const NEST: &[[u32; 10]; 10] = &[
    [7001, 7002, 7003, 7004, 7005, 7006, 7007, 7008, 7009, 7010],
    [7011, 7012, 7013, 7014, 7015, 7016, 7017, 7018, 7019, 7020],
    [7021, 7022, 7023, 7024, 7025, 7026, 7027, 7028, 7029, 7030],
    [7031, 7032, 7033, 7034, 7035, 7036, 7037, 7038, 7039, 7040],
    [7041, 7042, 7043, 7044, 7045, 7046, 7047, 7048, 7049, 7050],
    [7051, 7052, 7053, 7054, 7055, 7056, 7057, 7058, 7059, 7060],
    [7061, 7062, 7063, 7064, 7065, 7066, 7067, 7068, 7069, 7070],
    [7071, 7072, 7073, 7074, 7075, 7076, 7077, 7078, 7079, 7080],
    [7081, 7082, 7083, 7084, 7085, 7086, 7087, 7088, 7089, 7090],
    [7091, 7092, 7093, 7094, 7095, 7096, 7097, 7098, 7099, 7100]
];
const ENUMS: &[Shape] = &[
    Shape::Dot, Shape::Rect { w: 11, h: 12 }, Shape::Poly([21, 22, 23, 24, 25, 26]), Shape::Named("a1-shape-named-one", [31, 32, 33, 34]),
    Shape::Dot, Shape::Rect { w: 41, h: 42 }, Shape::Poly([51, 52, 53, 54, 55, 56]), Shape::Named("a1-shape-named-two", [61, 62, 63, 64]),
    Shape::Dot, Shape::Rect { w: 71, h: 72 }, Shape::Poly([81, 82, 83, 84, 85, 86]), Shape::Named("a1-shape-named-three", [91, 92, 93, 94]),
    Shape::Dot, Shape::Rect { w: 111, h: 112 }, Shape::Poly([121, 122, 123, 124, 125, 126]), Shape::Named("a1-shape-named-four", [131, 132, 133, 134]),
];
const BYVAL: [u32; 100] = [3001, 3002, 3003, 3004, 3005, 3006, 3007, 3008, 3009, 3010, 3011, 3012, 3013, 3014, 3015, 3016, 3017, 3018, 3019, 3020, 3021, 3022, 3023, 3024, 3025, 3026, 3027, 3028, 3029, 3030, 3031, 3032, 3033, 3034, 3035, 3036, 3037, 3038, 3039, 3040, 3041, 3042, 3043, 3044, 3045, 3046, 3047, 3048, 3049, 3050, 3051, 3052, 3053, 3054, 3055, 3056, 3057, 3058, 3059, 3060, 3061, 3062, 3063, 3064, 3065, 3066, 3067, 3068, 3069, 3070, 3071, 3072, 3073, 3074, 3075, 3076, 3077, 3078, 3079, 3080, 3081, 3082, 3083, 3084, 3085, 3086, 3087, 3088, 3089, 3090, 3091, 3092, 3093, 3094, 3095, 3096, 3097, 3098, 3099, 3100];
const BYROWS: [Row; 8] = [
    Row { name: "a1-byval-row-0-with-a-longish-name", weight: 10, pair: (100, 200), arr: [300, 400, 500] },
    Row { name: "a1-byval-row-1-with-a-longish-name", weight: 11, pair: (101, 201), arr: [301, 401, 501] },
    Row { name: "a1-byval-row-2-with-a-longish-name", weight: 12, pair: (102, 202), arr: [302, 402, 502] },
    Row { name: "a1-byval-row-3-with-a-longish-name", weight: 13, pair: (103, 203), arr: [303, 403, 503] },
    Row { name: "a1-byval-row-4-with-a-longish-name", weight: 14, pair: (104, 204), arr: [304, 404, 504] },
    Row { name: "a1-byval-row-5-with-a-longish-name", weight: 15, pair: (105, 205), arr: [305, 405, 505] },
    Row { name: "a1-byval-row-6-with-a-longish-name", weight: 16, pair: (106, 206), arr: [306, 406, 506] },
    Row { name: "a1-byval-row-7-with-a-longish-name", weight: 17, pair: (107, 207), arr: [307, 407, 507] }
];

#[inline(never)] fn f_deref_copy() {
    let mut a = *ARR; a[0] = 99; a[99] += 1;
    print_u32(a[0]); print_u32(ARR[0]); print_u32(ARR[99]);
    let r = black_box(ARR); let mut b = *r; b[1] = 77; print_u32(b[1]); print_u32(ARR[1]);
    let mut c = ARR.clone(); c[2] = 55; print_u32(c[2]); print_u32(ARR[2]);
    let mut d = *ARR; d.reverse(); print_u32(d[0]); print_u32(ARR[0]); d.sort_unstable(); d[3] = 1; print_u32(ARR[3]);
}
#[inline(never)] fn f_rows() {
    let mut r = ROWS[1]; r.weight = 999; r.arr[0] = 5; r.pair.0 = 7;
    print_u32(ROWS[1].weight); print_u32(ROWS[1].arr[0]); print_u32(ROWS[1].pair.0);
    for mut r in ROWS.iter().copied() { r.weight += 1; r.arr[1] = 9; r.pair.1 = 8; black_box(&mut r); }
    let mut all = *ROWARR; all[3].weight = 1; all[3].arr[2] = 42; all[4].pair = (9, 9); all[5].pair.0 = 3;
    print_u32(ROWARR[3].weight); print_u32(ROWARR[3].arr[2]); print_u32(ROWARR[4].pair.0); print_u32(ROWARR[5].pair.0);
    let r = Row { weight: 5, ..ROWS[0] }; let mut r2 = r; r2.arr[0] = 100; r2.pair.0 = 101; print_u32(ROWS[0].arr[0]); print_u32(ROWS[0].pair.0);
    let Row { name, mut weight, mut pair, mut arr } = ROWS[2]; arr[0] = 1; pair.1 = 2; weight += 1; black_box((&mut arr, &mut pair));
    print_u32(ROWS[2].arr[0]); print_u32(ROWS[2].pair.1); print_str(name);
    let mut first = *ROWS.first().unwrap(); first.arr[2] = 36; first.pair.0 = 1; print_u32(ROWS[0].arr[2]);
    let mut last = ROWS.last().copied().unwrap(); last.arr[2] = 37; print_u32(ROWS[7].arr[2]);
    let mut it = ROWS.iter().cloned(); let mut r = it.next().unwrap(); r.pair.0 = 37; r.arr[1] = 3; print_u32(ROWS[0].pair.0); print_u32(ROWS[0].arr[1]);
    let mut g = *ROWS.get(black_box(6)).unwrap(); g.arr[0] = 0; g.pair = (0, 0); print_u32(ROWS[6].arr[0]); print_u32(ROWS[6].pair.1);
    let mut arr_copy = ROWS[black_box(5)].arr; arr_copy[0] = 0; print_u32(ROWS[5].arr[0]);
    let mut pair_copy = ROWS[black_box(5)].pair; pair_copy.0 = 0; print_u32(ROWS[5].pair.0);
}
#[inline(never)] fn f_tuple_nested() {
    let (a, mut arr, mut row) = *TUP; arr[0] = 5; row.arr[0] = 6; row.pair.0 = 9; black_box((&mut arr, &mut row));
    print_u32(TUP.1[0]); print_u32(TUP.2.arr[0]); print_u32(TUP.2.pair.0); print_u32(a);
    let mut t = *TUP; t.1[5] = 1; t.2.weight = 3; t.2.arr[1] = 4; t.0 = 6; print_u32(TUP.1[5]); print_u32(TUP.2.weight); print_u32(TUP.2.arr[1]); print_u32(TUP.0);
    let mut n = *NEST; n[1][2] = 77; let mut inner = NEST[3]; inner[0] = 9; black_box((&mut n, &mut inner));
    print_u32(NEST[1][2]); print_u32(NEST[3][0]);
    let mut row0 = NEST[black_box(0)]; row0.reverse(); print_u32(NEST[0][0]);
}
#[inline(never)] fn f_enum(k: usize) {
    let mut s = ENUMS[k];
    match &mut s { Shape::Poly(p) => p[0] = 9, Shape::Rect { w, .. } => *w = 1, Shape::Named(_, a) => a[0] = 3, Shape::Dot => {} }
    black_box(&mut s);
    match &ENUMS[k] { Shape::Poly(p) => print_u32(p[0]), Shape::Rect { w, .. } => print_u32(*w), Shape::Named(n, a) => { print_str(n); print_u32(a[0]) }, Shape::Dot => print_str("dot") }
    if let Shape::Poly(mut p) = ENUMS[black_box(2)] { p[1] = 0; black_box(&mut p); }
    if let Shape::Poly(p) = &ENUMS[2] { print_u32(p[1]); }
}
#[inline(never)] fn f_replace() {
    let mut local = [0u32; 100];
    let old = core::mem::replace(&mut local, *ARR); local[5] = 1; print_u32(ARR[5]); print_u32(old[5]);
    let mut b = [1u32; 100]; core::mem::swap(&mut local, &mut b); b[6] = 2; local[7] = 3; print_u32(ARR[6]); print_u32(ARR[7]);
    let o: Option<&[u32; 100]> = black_box(Some(ARR)); let mut c = o.copied().unwrap(); c[7] = 3; print_u32(ARR[7]);
    let t = (ARR, SL); let mut d = *t.0; d[8] = 4; print_u32(ARR[8]); print_u32(t.1[8]);
    let mut rr = ROWS[0]; let old = core::mem::replace(&mut rr, ROWS[3]); rr.arr[0] = 0; rr.pair.0 = 0; print_u32(ROWS[3].arr[0]); print_u32(ROWS[3].pair.0); print_u32(old.weight);
    let mut cell = core::cell::Cell::new(ROWS[4]); let mut got = cell.get(); got.arr[0] = 0; cell.set(got); let m = cell.get_mut(); m.pair.0 = 0; print_u32(ROWS[4].arr[0]); print_u32(ROWS[4].pair.0);
    let rc = core::cell::RefCell::new(*ROWARR); rc.borrow_mut()[2].arr[0] = 0; rc.borrow_mut()[2].weight = 0; print_u32(ROWARR[2].arr[0]); print_u32(ROWARR[2].weight);
}
#[inline(never)] fn g_copy<T: Copy>(r: &T, f: impl Fn(&mut T)) -> T { let mut x = *r; f(&mut x); x }
#[inline(never)] fn f_generic() {
    let a = g_copy(ARR, |a| a[20] = 1); print_u32(a[20]); print_u32(ARR[20]);
    let r = g_copy(&ROWS[1], |r| { r.arr[2] = 1; r.pair.1 = 1; }); print_u32(r.arr[2]); print_u32(ROWS[1].arr[2]); print_u32(ROWS[1].pair.1);
    let t = g_copy(TUP, |t| { t.1[9] = 1; t.2.arr[2] = 1; }); print_u32(t.1[9]); print_u32(TUP.1[9]); print_u32(TUP.2.arr[2]);
    let n = g_copy(NEST, |n| n[9][9] = 1); print_u32(n[9][9]); print_u32(NEST[9][9]);
    let f = move |i: usize| { let mut x = *ARR; x[i] += 1; x[i] };
    print_u32(f(9)); print_u32(f(9)); print_u32(ARR[9]);
    let g = |i: usize| { let mut x = ROWS[i]; x.arr[0] += 1; x.pair.0 += 1; x.arr[0] + x.pair.0 };
    print_u32(g(7)); print_u32(g(7)); print_u32(ROWS[7].arr[0]);
}
#[inline(never)] fn f_into_iter() {
    let mut total = 0u32; for mut x in *ARR { x += 1; total = total.wrapping_add(x); } print_u32(total);
    let m = ARR.map(|x| x + 1); print_u32(m[0]); print_u32(ARR[0]);
    for mut r in *ROWARR { r.arr[0] = 77; r.pair.0 = 78; black_box(&mut r); } print_u32(ROWARR[0].arr[0]); print_u32(ROWARR[0].pair.0);
    let mut it = IntoIterator::into_iter(*ROWARR); let mut r = it.next().unwrap(); r.arr[2] = 66; let mut r2 = it.next_back().unwrap(); r2.pair.0 = 67; print_u32(ROWARR[0].arr[2]); print_u32(ROWARR[7].pair.0);
    let mut it2 = ROWARR.iter(); let x = it2.next().unwrap(); let mut y = *x; y.arr[0] = 1; print_u32(ROWARR[0].arr[0]);
    let mut z = ROWARR.iter().rev().copied().next().unwrap(); z.arr[0] = 1; print_u32(ROWARR[7].arr[0]);
    let mut z2 = ROWARR.iter().map(|r| r.arr).next().unwrap(); z2[0] = 2; print_u32(ROWARR[0].arr[0]);
    let mut mx = *ROWS.iter().max_by_key(|r| r.weight).unwrap(); mx.arr[0] = 0; print_u32(ROWS[7].arr[0]);
    let mut zipped = ROWS.iter().zip(ROWARR.iter()).map(|(a, b)| (*a, *b)).last().unwrap(); zipped.0.arr[0] = 0; zipped.1.arr[0] = 0; print_u32(ROWS[7].arr[0]); print_u32(ROWARR[7].arr[0]);
}
#[inline(never)] fn f_ptr() {
    let mut a = unsafe { core::ptr::read(ARR) }; a[10] = 1; print_u32(ARR[10]);
    let mut dst = [0u32; 100]; unsafe { core::ptr::copy_nonoverlapping(ARR.as_ptr(), dst.as_mut_ptr(), 100) }; dst[11] = 1; print_u32(ARR[11]);
    dst.copy_from_slice(SL); dst[12] = 1; print_u32(SL[12]);
    dst[..50].clone_from_slice(&SL[50..]); dst[0] = 1; print_u32(SL[50]);
    let mut rd = [ROWS[0]; 8]; rd[0].arr[0] = 30; rd[1].pair.0 = 30; print_u32(ROWS[0].arr[0]); print_u32(ROWS[0].pair.0);
    rd.copy_from_slice(ROWS); rd[1].arr[0] = 31; rd[1].weight = 32; rd[1].pair.0 = 33; print_u32(ROWS[1].arr[0]); print_u32(ROWS[1].weight); print_u32(ROWS[1].pair.0);
    rd.clone_from_slice(ROWS); rd[2].arr[0] = 33; rd[2].pair.1 = 34; print_u32(ROWS[2].arr[0]); print_u32(ROWS[2].pair.1);
    unsafe { core::ptr::copy_nonoverlapping(ROWS.as_ptr(), rd.as_mut_ptr(), 8) }; rd[3].arr[0] = 34; rd[3].pair.0 = 35; print_u32(ROWS[3].arr[0]); print_u32(ROWS[3].pair.0);
    unsafe { core::ptr::copy(ROWS.as_ptr().add(4), rd.as_mut_ptr(), 2) }; rd[0].arr[0] = 1; rd[1].pair.0 = 1; print_u32(ROWS[4].arr[0]); print_u32(ROWS[5].pair.0);
    let mut one = unsafe { core::ptr::read(&ROWS[4]) }; one.arr[0] = 35; one.pair.1 = 36; print_u32(ROWS[4].arr[0]); print_u32(ROWS[4].pair.1);
    let mut two = unsafe { ROWS.as_ptr().add(5).read() }; two.arr[1] = 0; two.pair.1 = 0; print_u32(ROWS[5].arr[1]); print_u32(ROWS[5].pair.1);
    let mut sw = *ROWARR; sw.swap(0, 7); sw[0].weight = 1; sw[0].arr[0] = 1; print_u32(ROWARR[7].weight); print_u32(ROWARR[7].arr[0]);
    sw.sort_unstable_by_key(|r| r.weight); sw[0].arr[0] = 44; sw[7].pair.0 = 45; sw.reverse(); sw[3].arr[1] = 46;
    let mut s0 = 0; let mut s1 = 0; let mut s2 = 0; for r in ROWARR { s0 += r.arr[0]; s1 += r.arr[1]; s2 += r.pair.0; } print_u32(s0); print_u32(s1); print_u32(s2);
    let mut fill = [ROWS[0]; 4]; fill.fill(ROWS[2]); fill[0].arr[0] = 0; fill[1].arr[0] = 5; print_u32(ROWS[2].arr[0]); print_u32(fill[1].arr[0]); print_u32(fill[2].arr[0]);
}
#[inline(never)] fn f_byval() {
    let s: u32 = BYVAL.iter().sum(); print_u32(s);
    let mut c = BYVAL; c[0] = 1; c[50] = 2; print_u32(BYVAL[0]); print_u32(BYVAL.iter().sum::<u32>());
    let r = &BYVAL; let mut d = *r; d[1] = 5; print_u32(BYVAL[1]); print_u32(r[1]);
    let w: u32 = BYROWS.iter().map(|r| r.weight + r.arr[0]).sum(); print_u32(w);
    let mut e = BYROWS; e[0].arr[0] = 0; e[1].weight = 0; let rr = &BYROWS; let mut f = *rr; f[2].arr[0] = 0; let mut g = rr[3]; g.arr[0] = 0; g.pair.0 = 0;
    print_u32(BYROWS.iter().map(|r| r.weight + r.arr[0] + r.pair.0).sum::<u32>());
}
#[inline(never)] fn checksum() {
    let mut s = 0u32;
    for x in ARR { s = s.wrapping_mul(31).wrapping_add(*x); }
    for x in SL { s = s.wrapping_mul(31).wrapping_add(*x); }
    for r in ROWS { s = s.wrapping_mul(31).wrapping_add(r.weight + r.pair.0 * 3 + r.pair.1 * 5 + r.arr[0] * 7 + r.arr[1] * 11 + r.arr[2] * 13 + r.name.len() as u32); }
    for r in ROWARR { s = s.wrapping_mul(31).wrapping_add(r.weight + r.pair.0 * 3 + r.pair.1 * 5 + r.arr[0] * 7 + r.arr[1] * 11 + r.arr[2] * 13 + r.name.len() as u32); }
    s = s.wrapping_mul(31).wrapping_add(TUP.0); for x in &TUP.1 { s = s.wrapping_mul(31).wrapping_add(*x); }
    s = s.wrapping_mul(31).wrapping_add(TUP.2.weight + TUP.2.arr[0] + TUP.2.arr[1] + TUP.2.arr[2] + TUP.2.pair.0 + TUP.2.pair.1);
    for row in NEST { for x in row { s = s.wrapping_mul(31).wrapping_add(*x); } }
    for e in ENUMS { s = s.wrapping_mul(31).wrapping_add(match e { Shape::Dot => 1, Shape::Rect { w, h } => w * 3 + h, Shape::Poly(p) => p.iter().sum::<u32>(), Shape::Named(n, a) => n.len() as u32 + a.iter().sum::<u32>() }); }
    for x in BYVAL.iter() { s = s.wrapping_mul(31).wrapping_add(*x); }
    for r in BYROWS.iter() { s = s.wrapping_mul(31).wrapping_add(r.weight + r.pair.0 + r.arr[0]); }
    print_u32(s);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    checksum();
    for round in 0..2 {
        f_deref_copy(); f_rows(); f_tuple_nested();
        for k in 0..16 { f_enum(black_box(k)); }
        f_replace(); f_generic(); f_into_iter(); f_ptr(); f_byval();
        checksum();
    }
}
