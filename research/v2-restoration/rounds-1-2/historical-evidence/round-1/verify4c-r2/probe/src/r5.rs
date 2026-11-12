//! Verifier probe: which written-but-never-read copies of pooled data write through.
#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
#[derive(Clone, Copy)]
struct Row { name: &'static str, weight: u32, arr: [u32; 2] }
const TUPS: &[(u32, u32)] = &[(0, 1000), (1, 1007), (2, 1014), (3, 1021), (4, 1028), (5, 1035), (6, 1042), (7, 1049), (8, 1056), (9, 1063), (10, 1070), (11, 1077), (12, 1084), (13, 1091), (14, 1098), (15, 1105), (16, 1112), (17, 1119), (18, 1126), (19, 1133), (20, 1140), (21, 1147), (22, 1154), (23, 1161), (24, 1168), (25, 1175), (26, 1182), (27, 1189), (28, 1196), (29, 1203), (30, 1210), (31, 1217), (32, 1224), (33, 1231), (34, 1238), (35, 1245), (36, 1252), (37, 1259), (38, 1266), (39, 1273)];
const ARRS: &[[u32; 2]] = &[[0, 2000], [1, 2007], [2, 2014], [3, 2021], [4, 2028], [5, 2035], [6, 2042], [7, 2049], [8, 2056], [9, 2063], [10, 2070], [11, 2077], [12, 2084], [13, 2091], [14, 2098], [15, 2105], [16, 2112], [17, 2119], [18, 2126], [19, 2133], [20, 2140], [21, 2147], [22, 2154], [23, 2161], [24, 2168], [25, 2175], [26, 2182], [27, 2189], [28, 2196], [29, 2203], [30, 2210], [31, 2217], [32, 2224], [33, 2231], [34, 2238], [35, 2245], [36, 2252], [37, 2259], [38, 2266], [39, 2273]];
const ARR: &[u32; 100] = &[1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
const ROWS: &[Row] = &[Row { name: "r5-row-0-with-a-longish-name", weight: 10, arr: [0, 1] }, Row { name: "r5-row-1-with-a-longish-name", weight: 11, arr: [1, 2] }, Row { name: "r5-row-2-with-a-longish-name", weight: 12, arr: [2, 3] }, Row { name: "r5-row-3-with-a-longish-name", weight: 13, arr: [3, 4] }, Row { name: "r5-row-4-with-a-longish-name", weight: 14, arr: [4, 5] }, Row { name: "r5-row-5-with-a-longish-name", weight: 15, arr: [5, 6] }, Row { name: "r5-row-6-with-a-longish-name", weight: 16, arr: [6, 7] }, Row { name: "r5-row-7-with-a-longish-name", weight: 17, arr: [7, 8] }, Row { name: "r5-row-8-with-a-longish-name", weight: 18, arr: [8, 9] }, Row { name: "r5-row-9-with-a-longish-name", weight: 19, arr: [9, 10] }];
const NEST: &[[u32; 2]; 40] = &[[0, 2000], [1, 2007], [2, 2014], [3, 2021], [4, 2028], [5, 2035], [6, 2042], [7, 2049], [8, 2056], [9, 2063], [10, 2070], [11, 2077], [12, 2084], [13, 2091], [14, 2098], [15, 2105], [16, 2112], [17, 2119], [18, 2126], [19, 2133], [20, 2140], [21, 2147], [22, 2154], [23, 2161], [24, 2168], [25, 2175], [26, 2182], [27, 2189], [28, 2196], [29, 2203], [30, 2210], [31, 2217], [32, 2224], [33, 2231], [34, 2238], [35, 2245], [36, 2252], [37, 2259], [38, 2266], [39, 2273]];
#[inline(never)] fn sums() {
    print_u32(TUPS.iter().map(|t| t.0).sum()); print_u32(ARRS.iter().map(|a| a[0]).sum()); print_u32(ARR.iter().sum());
    print_u32(ROWS.iter().map(|r| r.weight + r.arr[0]).sum()); print_u32(NEST.iter().map(|a| a[0]).sum());
}
#[inline(always)] fn ignore(_r: &Row) {}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    sums();
    print_str("tuple element"); let mut t = TUPS[3]; t.0 = 77; print_u32(TUPS[3].0);
    print_str("array element"); let mut a = ARRS[3]; a[0] = 77; print_u32(ARRS[3][0]);
    print_str("whole array deref"); let mut w = *ARR; w[0] = 77; print_u32(ARR[0]);
    print_str("row with nested array"); let mut r = ROWS[2]; r.weight = 77; r.arr[0] = 78; print_u32(ROWS[2].weight); print_u32(ROWS[2].arr[0]);
    print_str("row passed to an inlined no-op"); let mut q = ROWS[3]; q.weight += 100; ignore(&q); print_u32(ROWS[3].weight);
    print_str("nested array element"); let mut n = NEST[5]; n[1] = 79; print_u32(NEST[5][1]);
    print_str("dynamic index in a loop"); for i in 0..3 { let mut t = TUPS[black_box(i)]; t.1 += 1; } print_u32(TUPS[0].1 + TUPS[1].1 + TUPS[2].1);
    sums();
}
