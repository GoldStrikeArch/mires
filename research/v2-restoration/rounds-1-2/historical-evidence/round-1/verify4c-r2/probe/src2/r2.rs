//! Verifier probe: variants of copying an element out of a pooled table and writing the copy.
#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;

#[derive(Clone, Copy)]
struct P { x: u32, y: u32 }
#[derive(Clone, Copy)]
struct Row { name: &'static str, weight: u32 }
#[derive(Clone, Copy)]
struct Cfg { p: P, limits: [u32; 80], t: (u32, u32) }
const PS1: &[P] = &[P { x: 100, y: 5100 }, P { x: 101, y: 5101 }, P { x: 102, y: 5102 }, P { x: 103, y: 5103 }, P { x: 104, y: 5104 }, P { x: 105, y: 5105 }, P { x: 106, y: 5106 }, P { x: 107, y: 5107 }, P { x: 108, y: 5108 }, P { x: 109, y: 5109 }, P { x: 110, y: 5110 }, P { x: 111, y: 5111 }, P { x: 112, y: 5112 }, P { x: 113, y: 5113 }, P { x: 114, y: 5114 }, P { x: 115, y: 5115 }, P { x: 116, y: 5116 }, P { x: 117, y: 5117 }, P { x: 118, y: 5118 }, P { x: 119, y: 5119 }, P { x: 120, y: 5120 }, P { x: 121, y: 5121 }, P { x: 122, y: 5122 }, P { x: 123, y: 5123 }];
const PS2: &[P] = &[P { x: 200, y: 5200 }, P { x: 201, y: 5201 }, P { x: 202, y: 5202 }, P { x: 203, y: 5203 }, P { x: 204, y: 5204 }, P { x: 205, y: 5205 }, P { x: 206, y: 5206 }, P { x: 207, y: 5207 }, P { x: 208, y: 5208 }, P { x: 209, y: 5209 }, P { x: 210, y: 5210 }, P { x: 211, y: 5211 }, P { x: 212, y: 5212 }, P { x: 213, y: 5213 }, P { x: 214, y: 5214 }, P { x: 215, y: 5215 }, P { x: 216, y: 5216 }, P { x: 217, y: 5217 }, P { x: 218, y: 5218 }, P { x: 219, y: 5219 }, P { x: 220, y: 5220 }, P { x: 221, y: 5221 }, P { x: 222, y: 5222 }, P { x: 223, y: 5223 }];
const PS3: &[P] = &[P { x: 300, y: 5300 }, P { x: 301, y: 5301 }, P { x: 302, y: 5302 }, P { x: 303, y: 5303 }, P { x: 304, y: 5304 }, P { x: 305, y: 5305 }, P { x: 306, y: 5306 }, P { x: 307, y: 5307 }, P { x: 308, y: 5308 }, P { x: 309, y: 5309 }, P { x: 310, y: 5310 }, P { x: 311, y: 5311 }, P { x: 312, y: 5312 }, P { x: 313, y: 5313 }, P { x: 314, y: 5314 }, P { x: 315, y: 5315 }, P { x: 316, y: 5316 }, P { x: 317, y: 5317 }, P { x: 318, y: 5318 }, P { x: 319, y: 5319 }, P { x: 320, y: 5320 }, P { x: 321, y: 5321 }, P { x: 322, y: 5322 }, P { x: 323, y: 5323 }];
const PS6: &[P] = &[P { x: 600, y: 5600 }, P { x: 601, y: 5601 }, P { x: 602, y: 5602 }, P { x: 603, y: 5603 }, P { x: 604, y: 5604 }, P { x: 605, y: 5605 }, P { x: 606, y: 5606 }, P { x: 607, y: 5607 }, P { x: 608, y: 5608 }, P { x: 609, y: 5609 }, P { x: 610, y: 5610 }, P { x: 611, y: 5611 }, P { x: 612, y: 5612 }, P { x: 613, y: 5613 }, P { x: 614, y: 5614 }, P { x: 615, y: 5615 }, P { x: 616, y: 5616 }, P { x: 617, y: 5617 }, P { x: 618, y: 5618 }, P { x: 619, y: 5619 }, P { x: 620, y: 5620 }, P { x: 621, y: 5621 }, P { x: 622, y: 5622 }, P { x: 623, y: 5623 }];
const PS7: &[P] = &[P { x: 700, y: 5700 }, P { x: 701, y: 5701 }, P { x: 702, y: 5702 }, P { x: 703, y: 5703 }, P { x: 704, y: 5704 }, P { x: 705, y: 5705 }, P { x: 706, y: 5706 }, P { x: 707, y: 5707 }, P { x: 708, y: 5708 }, P { x: 709, y: 5709 }, P { x: 710, y: 5710 }, P { x: 711, y: 5711 }, P { x: 712, y: 5712 }, P { x: 713, y: 5713 }, P { x: 714, y: 5714 }, P { x: 715, y: 5715 }, P { x: 716, y: 5716 }, P { x: 717, y: 5717 }, P { x: 718, y: 5718 }, P { x: 719, y: 5719 }, P { x: 720, y: 5720 }, P { x: 721, y: 5721 }, P { x: 722, y: 5722 }, P { x: 723, y: 5723 }];
const PARR: &[P; 24] = &[P { x: 800, y: 5800 }, P { x: 801, y: 5801 }, P { x: 802, y: 5802 }, P { x: 803, y: 5803 }, P { x: 804, y: 5804 }, P { x: 805, y: 5805 }, P { x: 806, y: 5806 }, P { x: 807, y: 5807 }, P { x: 808, y: 5808 }, P { x: 809, y: 5809 }, P { x: 810, y: 5810 }, P { x: 811, y: 5811 }, P { x: 812, y: 5812 }, P { x: 813, y: 5813 }, P { x: 814, y: 5814 }, P { x: 815, y: 5815 }, P { x: 816, y: 5816 }, P { x: 817, y: 5817 }, P { x: 818, y: 5818 }, P { x: 819, y: 5819 }, P { x: 820, y: 5820 }, P { x: 821, y: 5821 }, P { x: 822, y: 5822 }, P { x: 823, y: 5823 }];
const TUPS: &[(u32, u32)] = &[(0, 1000), (1, 1007), (2, 1014), (3, 1021), (4, 1028), (5, 1035), (6, 1042), (7, 1049), (8, 1056), (9, 1063), (10, 1070), (11, 1077), (12, 1084), (13, 1091), (14, 1098), (15, 1105), (16, 1112), (17, 1119), (18, 1126), (19, 1133), (20, 1140), (21, 1147), (22, 1154), (23, 1161), (24, 1168), (25, 1175), (26, 1182), (27, 1189), (28, 1196), (29, 1203), (30, 1210), (31, 1217), (32, 1224), (33, 1231), (34, 1238), (35, 1245), (36, 1252), (37, 1259), (38, 1266), (39, 1273)];
const ARRS: &[[u32; 2]] = &[[0, 2000], [1, 2007], [2, 2014], [3, 2021], [4, 2028], [5, 2035], [6, 2042], [7, 2049], [8, 2056], [9, 2063], [10, 2070], [11, 2077], [12, 2084], [13, 2091], [14, 2098], [15, 2105], [16, 2112], [17, 2119], [18, 2126], [19, 2133], [20, 2140], [21, 2147], [22, 2154], [23, 2161], [24, 2168], [25, 2175], [26, 2182], [27, 2189], [28, 2196], [29, 2203], [30, 2210], [31, 2217], [32, 2224], [33, 2231], [34, 2238], [35, 2245], [36, 2252], [37, 2259], [38, 2266], [39, 2273]];
const ROWS: &[Row] = &[Row { name: "r2-row-0-with-a-longish-name", weight: 10 }, Row { name: "r2-row-1-with-a-longish-name", weight: 11 }, Row { name: "r2-row-2-with-a-longish-name", weight: 12 }, Row { name: "r2-row-3-with-a-longish-name", weight: 13 }, Row { name: "r2-row-4-with-a-longish-name", weight: 14 }, Row { name: "r2-row-5-with-a-longish-name", weight: 15 }, Row { name: "r2-row-6-with-a-longish-name", weight: 16 }, Row { name: "r2-row-7-with-a-longish-name", weight: 17 }, Row { name: "r2-row-8-with-a-longish-name", weight: 18 }, Row { name: "r2-row-9-with-a-longish-name", weight: 19 }];
const CFG: &Cfg = &Cfg { p: P { x: 1, y: 2 }, limits: [9000, 9001, 9002, 9003, 9004, 9005, 9006, 9007, 9008, 9009, 9010, 9011, 9012, 9013, 9014, 9015, 9016, 9017, 9018, 9019, 9020, 9021, 9022, 9023, 9024, 9025, 9026, 9027, 9028, 9029, 9030, 9031, 9032, 9033, 9034, 9035, 9036, 9037, 9038, 9039, 9040, 9041, 9042, 9043, 9044, 9045, 9046, 9047, 9048, 9049, 9050, 9051, 9052, 9053, 9054, 9055, 9056, 9057, 9058, 9059, 9060, 9061, 9062, 9063, 9064, 9065, 9066, 9067, 9068, 9069, 9070, 9071, 9072, 9073, 9074, 9075, 9076, 9077, 9078, 9079], t: (3, 4) };
const BYVAL: [P; 24] = [P { x: 900, y: 5900 }, P { x: 901, y: 5901 }, P { x: 902, y: 5902 }, P { x: 903, y: 5903 }, P { x: 904, y: 5904 }, P { x: 905, y: 5905 }, P { x: 906, y: 5906 }, P { x: 907, y: 5907 }, P { x: 908, y: 5908 }, P { x: 909, y: 5909 }, P { x: 910, y: 5910 }, P { x: 911, y: 5911 }, P { x: 912, y: 5912 }, P { x: 913, y: 5913 }, P { x: 914, y: 5914 }, P { x: 915, y: 5915 }, P { x: 916, y: 5916 }, P { x: 917, y: 5917 }, P { x: 918, y: 5918 }, P { x: 919, y: 5919 }, P { x: 920, y: 5920 }, P { x: 921, y: 5921 }, P { x: 922, y: 5922 }, P { x: 923, y: 5923 }];

#[inline(never)] fn bump(i: usize) -> u32 { let mut p = PS3[i]; p.x += 100; p.x }
#[inline(never)] fn bump_row(i: usize) -> u32 { let mut r = ROWS[i]; r.weight += 100; r.weight }
#[inline(never)] fn sums() {
    print_u32(PS1.iter().map(|p| p.x).sum()); print_u32(PS2.iter().map(|p| p.x).sum()); print_u32(PS3.iter().map(|p| p.x).sum());
    print_u32(PS6.iter().map(|p| p.x).sum()); print_u32(PS7.iter().map(|p| p.x).sum()); print_u32(PARR.iter().map(|p| p.x).sum());
    print_u32(TUPS.iter().map(|t| t.0).sum()); print_u32(ARRS.iter().map(|a| a[0]).sum()); print_u32(ROWS.iter().map(|r| r.weight).sum());
    print_u32(CFG.p.x + CFG.t.0 + CFG.limits[0]); print_u32(BYVAL.iter().map(|p| p.x).sum());
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    sums();
    print_str("V1 dead copy"); let mut p = PS1[4]; p.x = 99; print_u32(PS1[4].x);
    print_str("V2 copy read after write"); let mut p = PS2[4]; p.x = 99; print_u32(p.x); print_u32(PS2[4].x);
    print_str("V3 in a function, twice"); print_u32(bump(5)); print_u32(bump(5)); print_u32(PS3[5].x);
    print_str("V4 tuple element"); let mut t = TUPS[3]; t.0 = 77; print_u32(t.0); print_u32(TUPS[3].0);
    print_str("V5 array element"); let mut a = ARRS[3]; a[0] = 77; print_u32(a[0]); print_u32(ARRS[3][0]);
    print_str("V6 iterator copied"); let mut n = 0; for mut p in PS6.iter().copied() { p.x += 1; n += p.x; } print_u32(n); print_u32(PS6[0].x);
    print_str("V7 get/first"); let mut g = *PS7.get(black_box(2)).unwrap(); g.x = 55; let mut f = *PS7.first().unwrap(); f.y = 56; print_u32(g.x + f.y); print_u32(PS7[2].x); print_u32(PS7[0].y);
    print_str("V8 array reference element"); let mut p = PARR[4]; p.x = 99; print_u32(p.x); print_u32(PARR[4].x);
    print_str("V9 row with str, in a function, twice"); print_u32(bump_row(1)); print_u32(bump_row(1)); print_u32(ROWS[1].weight);
    print_str("V10 field of a referenced struct"); let mut p = CFG.p; p.x = 5; let mut t = CFG.t; t.0 = 6; print_u32(p.x + t.0); print_u32(CFG.p.x); print_u32(CFG.t.0);
    print_str("V11 dynamic index"); let i = black_box(7); let mut p = PS1[i]; p.y = 1; print_u32(p.y); print_u32(PS1[7].y);
    print_str("V12 by-value const element"); let mut p = BYVAL[4]; p.x = 99; print_u32(p.x); print_u32(BYVAL[4].x);
    print_str("V13 whole deref of referenced struct"); let mut c = *CFG; c.p.x = 9; c.limits[0] = 9; c.t.1 = 9; print_u32(c.p.x); print_u32(CFG.p.x + CFG.limits[0] + CFG.t.1);
    sums();
}
