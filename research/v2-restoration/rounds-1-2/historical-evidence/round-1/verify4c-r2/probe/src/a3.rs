
#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::ptr;

#[derive(Clone, Copy, PartialEq)]
struct Row { name: &'static str, weight: u32, pair: (u32, u32), arr: [u32; 3] }

const A: &[u32; 100] = &[1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
const A2: &[u32; 100] = &[1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
const S1: &[u32] = &[2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2009, 2010, 2011, 2012, 2013, 2014, 2015, 2016, 2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026, 2027, 2028, 2029, 2030, 2031, 2032, 2033, 2034, 2035, 2036, 2037, 2038, 2039, 2040, 2041, 2042, 2043, 2044, 2045, 2046, 2047, 2048, 2049, 2050, 2051, 2052, 2053, 2054, 2055, 2056, 2057, 2058, 2059, 2060, 2061, 2062, 2063, 2064, 2065, 2066, 2067, 2068, 2069, 2070, 2071, 2072, 2073, 2074, 2075, 2076, 2077, 2078, 2079, 2080, 2081, 2082, 2083, 2084, 2085, 2086, 2087, 2088, 2089, 2090, 2091, 2092, 2093, 2094, 2095, 2096, 2097, 2098, 2099, 2100];
const S2: &[u32] = &[2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2009, 2010, 2011, 2012, 2013, 2014, 2015, 2016, 2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026, 2027, 2028, 2029, 2030, 2031, 2032, 2033, 2034, 2035, 2036, 2037, 2038, 2039, 2040, 2041, 2042, 2043, 2044, 2045, 2046, 2047, 2048, 2049, 2050, 2051, 2052, 2053, 2054, 2055, 2056, 2057, 2058, 2059, 2060, 2061, 2062, 2063, 2064, 2065, 2066, 2067, 2068, 2069, 2070, 2071, 2072, 2073, 2074, 2075, 2076, 2077, 2078, 2079, 2080, 2081, 2082, 2083, 2084, 2085, 2086, 2087, 2088, 2089, 2090, 2091, 2092, 2093, 2094, 2095, 2096, 2097, 2098, 2099, 2100];
static ST: [u32; 100] = [1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
static ST2: [u32; 100] = [1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
static SREF: &[u32; 100] = &[1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
static SSL: &[u32] = &[2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2009, 2010, 2011, 2012, 2013, 2014, 2015, 2016, 2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026, 2027, 2028, 2029, 2030, 2031, 2032, 2033, 2034, 2035, 2036, 2037, 2038, 2039, 2040, 2041, 2042, 2043, 2044, 2045, 2046, 2047, 2048, 2049, 2050, 2051, 2052, 2053, 2054, 2055, 2056, 2057, 2058, 2059, 2060, 2061, 2062, 2063, 2064, 2065, 2066, 2067, 2068, 2069, 2070, 2071, 2072, 2073, 2074, 2075, 2076, 2077, 2078, 2079, 2080, 2081, 2082, 2083, 2084, 2085, 2086, 2087, 2088, 2089, 2090, 2091, 2092, 2093, 2094, 2095, 2096, 2097, 2098, 2099, 2100];
static mut COUNTERS: [u32; 100] = [1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
const ROWS: &[Row] = &[
    Row { name: "a3-slice-row-0-with-a-longish-name", weight: 10, pair: (100, 200), arr: [300, 400, 500] },
    Row { name: "a3-slice-row-1-with-a-longish-name", weight: 11, pair: (101, 201), arr: [301, 401, 501] },
    Row { name: "a3-slice-row-2-with-a-longish-name", weight: 12, pair: (102, 202), arr: [302, 402, 502] },
    Row { name: "a3-slice-row-3-with-a-longish-name", weight: 13, pair: (103, 203), arr: [303, 403, 503] },
    Row { name: "a3-slice-row-4-with-a-longish-name", weight: 14, pair: (104, 204), arr: [304, 404, 504] },
    Row { name: "a3-slice-row-5-with-a-longish-name", weight: 15, pair: (105, 205), arr: [305, 405, 505] },
    Row { name: "a3-slice-row-6-with-a-longish-name", weight: 16, pair: (106, 206), arr: [306, 406, 506] },
    Row { name: "a3-slice-row-7-with-a-longish-name", weight: 17, pair: (107, 207), arr: [307, 407, 507] }
];
static INDEX: &[Row] = ROWS;

fn label(s: &str, b: bool) { print_str(s); print_bool(b); }
#[inline(never)] fn addr_a() -> usize { A.as_ptr() as usize }
#[inline(never)] fn addr_s1() -> usize { S1.as_ptr() as usize }

#[inline(never)] fn defined() {
    // Comparisons whose result Rust defines.
    let r = black_box(A);
    label("D01 reflexive ref", ptr::eq(r, r));
    label("D02 distinct statics", ptr::eq(&ST, &ST2));
    label("D03 static reflexive", ptr::eq(&ST, &ST));
    let local = *A;
    label("D04 local copy vs const", ptr::eq(&local, r));
    let s = black_box(S1);
    print_str("D05 subslice offset"); print_usize((s[1..].as_ptr() as usize) - (s.as_ptr() as usize));
    let (l, rr) = s.split_at(50);
    print_str("D06 split offset"); print_usize(rr.as_ptr() as usize - l.as_ptr() as usize);
    print_str("D07 offset_from"); print_i32(unsafe { rr.as_ptr().offset_from(l.as_ptr()) } as i32);
    let range = s.as_ptr_range();
    print_str("D08 ptr range"); print_usize(range.end as usize - range.start as usize);
    label("D09 element eq", ptr::eq(&s[3], &s[3..][0]));
    label("D10 element vs add", &s[3] as *const u32 == s.as_ptr().wrapping_add(3));
    let rows = black_box(ROWS);
    label("D11 row element eq", ptr::eq(&rows[1], &rows[1..][0]));
    label("D12 row stride", (&rows[2] as *const Row as usize) - (&rows[1] as *const Row as usize) == core::mem::size_of::<Row>());
    label("D13 field offset", (&rows[1].weight as *const u32 as usize) - (&rows[1] as *const Row as usize) == core::mem::offset_of!(Row, weight));
    let sl: &[u32] = r;
    label("D14 unsize keeps address", sl.as_ptr() == r.as_ptr());
    label("D15 array ptr vs first", r as *const [u32; 100] as *const u32 == &r[0] as *const u32);
    label("D16 iter slice ptr", r.iter().as_slice().as_ptr() == r.as_ptr());
    label("D17 const vs distinct static (content equal)", ptr::eq(&ST, &ST2) || ptr::eq(&ST as *const [u32; 100], unsafe { ptr::addr_of!(COUNTERS) }));
    label("D18 rows differ", ptr::eq(&rows[0], &rows[1]));
    label("D19 one-past-end", s[100..].as_ptr() == s.as_ptr().wrapping_add(100));
    label("D20 chunks ptr", s.chunks(10).nth(3).unwrap().as_ptr() == s.as_ptr().wrapping_add(30));
    label("D21 windows ptr", s.windows(4).nth(7).unwrap().as_ptr() == &s[7] as *const u32);
    label("D22 first/last", ptr::eq(s.first().unwrap(), &s[0]) && ptr::eq(s.last().unwrap(), &s[99]));
    label("D23 from_raw_parts", unsafe { core::slice::from_raw_parts(s.as_ptr().add(5), 10) } == &s[5..15]);
    label("D24 slice eq content", S1 == S2 && A == A2 && S1[10..20] == S2[10..20] && *A == ST);
    // Writing a static mut with the constant's contents never changes the constant.
    unsafe { COUNTERS[0] = 5; COUNTERS[99] += 1; }
    print_str("D25 const after static mut write"); print_u32(A[0]); print_u32(A[99]); print_u32(A2[0]); print_u32(ST[0]); print_u32(SREF[0]); print_u32(unsafe { COUNTERS[0] });
    label("D26 static mut distinct", ptr::eq(r, unsafe { ptr::addr_of!(COUNTERS) }));
    label("D27 addr nonzero aligned", (r.as_ptr() as usize) != 0 && (r.as_ptr() as usize) % 4 == 0);
    label("D28 const vs heap-free local arr", ptr::eq(&[0u32; 100], r));
}
#[inline(never)] fn unspecified() {
    // Comparisons Rust leaves unspecified: printed for the record.
    label("U01 two uses of one const", ptr::eq(A, A));
    label("U02 two consts same content", ptr::eq(A, A2));
    label("U03 const vs static same content", ptr::eq(A, &ST));
    label("U04 static-held ref vs const", ptr::eq(SREF, A));
    label("U05 static-held ref vs static", ptr::eq(SREF, &ST));
    label("U06 slice two uses", S1.as_ptr() == S1.as_ptr());
    label("U07 slices same content", S1.as_ptr() == S2.as_ptr());
    label("U08 static-held slice vs const", SSL.as_ptr() == S1.as_ptr());
    label("U09 addr stable across calls", addr_a() == addr_a());
    label("U10 slice addr stable across calls", addr_s1() == addr_s1());
    label("U11 static copy of const ref", ptr::eq(INDEX, ROWS));
    label("U12 ordering total", (A.as_ptr() < A2.as_ptr()) || (A.as_ptr() >= A2.as_ptr()));
    print_str("U13 cross-use offset is stride"); print_bool((S1[1..].as_ptr() as usize).wrapping_sub(S1.as_ptr() as usize) == 4);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    defined();
    unspecified();
    defined();
}
