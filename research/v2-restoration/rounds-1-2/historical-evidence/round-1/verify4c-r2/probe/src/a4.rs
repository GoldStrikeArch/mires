
#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cell::Cell;
use core::hint::black_box;

#[derive(Clone, Copy, PartialEq)]
struct Row { name: &'static str, weight: u32, pair: (u32, u32), arr: [u32; 3] }
const ROWS: &[Row] = &[
    Row { name: "a4-slice-row-0-with-a-longish-name", weight: 10, pair: (100, 200), arr: [300, 400, 500] },
    Row { name: "a4-slice-row-1-with-a-longish-name", weight: 11, pair: (101, 201), arr: [301, 401, 501] },
    Row { name: "a4-slice-row-2-with-a-longish-name", weight: 12, pair: (102, 202), arr: [302, 402, 502] },
    Row { name: "a4-slice-row-3-with-a-longish-name", weight: 13, pair: (103, 203), arr: [303, 403, 503] },
    Row { name: "a4-slice-row-4-with-a-longish-name", weight: 14, pair: (104, 204), arr: [304, 404, 504] },
    Row { name: "a4-slice-row-5-with-a-longish-name", weight: 15, pair: (105, 205), arr: [305, 405, 505] },
    Row { name: "a4-slice-row-6-with-a-longish-name", weight: 16, pair: (106, 206), arr: [306, 406, 506] },
    Row { name: "a4-slice-row-7-with-a-longish-name", weight: 17, pair: (107, 207), arr: [307, 407, 507] }
];
const LONG: &[(&str, &str)] = &[
    ("a4-key-0", "this value is definitely longer than thirty-two bytes, number 0"),
    ("a4-key-1", "this value is definitely longer than thirty-two bytes, number 1"),
    ("a4-key-2", "this value is definitely longer than thirty-two bytes, number 2"),
    ("a4-key-3", "this value is definitely longer than thirty-two bytes, number 3"),
    ("a4-key-4", "this value is definitely longer than thirty-two bytes, number 4"),
    ("a4-key-5", "this value is definitely longer than thirty-two bytes, number 5"),
    ("a4-key-6", "this value is definitely longer than thirty-two bytes, number 6"),
    ("a4-key-7", "this value is definitely longer than thirty-two bytes, number 7")
];
// Statics evaluated at module load that read pooled data (directly, nested, and inside interior mutability).
struct Holder { table: &'static [(&'static str, &'static str)], rows: &'static [Row], grid: [&'static [Row]; 2], first: &'static Row }
static LONG_STATIC: &[(&str, &str)] = LONG;
static FIRST_LONG: &(&str, &str) = &LONG[0];
static HOLDER: Holder = Holder { table: LONG, rows: ROWS, grid: [ROWS, ROWS], first: &ROWS[3] };
struct Shared(Cell<&'static [Row]>);
unsafe impl Sync for Shared {}
static CELLED: Shared = Shared(Cell::new(ROWS));
static AAA_EARLY: &[Row] = ROWS; // sorts before every other item name
static ZZZ_LATE: (&[Row], &[(&str, &str)]) = (ROWS, LONG);

// Functions reachable only through a pooled table: plain fns, a generic instance, closures.
#[inline(never)] fn sum(s: &[u32]) -> u32 { s.iter().sum() }
#[inline(never)] fn max_of(s: &[u32]) -> u32 { s.iter().copied().max().unwrap_or(0) }
#[inline(never)] fn first<T: Copy + Into<u32>>(s: &[T]) -> u32 { s.first().map_or(0, |x| (*x).into()) }
#[inline(never)] fn nested_reader(s: &[u32]) -> u32 { s.len() as u32 + ROWS.len() as u32 + LONG[1].1.len() as u32 }
const OPS: &[(&str, fn(&[u32]) -> u32)] = &[
    ("a4-op-sum-with-a-long-name", sum), ("a4-op-max-with-a-long-name", max_of), ("a4-op-first-with-a-long-name", first::<u32>),
    ("a4-op-len-closure-long-name", max_of), ("a4-op-last-closure-long-name", first::<u32>),
    ("a4-op-nested-reader-long-name", nested_reader), ("a4-op-sum-again-with-long-name", sum), ("a4-op-xor-closure-long-name", nested_reader),
];
static OPS_STATIC: &[(&str, fn(&[u32]) -> u32)] = OPS;

// A by-value const whose inner references are large: the struct is per use, the inner data shared.
#[derive(Clone, Copy)]
struct Config { rows: &'static [Row], nums: &'static [u32; 80], label: &'static str, counts: [u32; 4] }
const CFG: Config = Config { rows: &[
    Row { name: "a4-config-row-0-with-a-longish-name", weight: 10, pair: (100, 200), arr: [300, 400, 500] },
    Row { name: "a4-config-row-1-with-a-longish-name", weight: 11, pair: (101, 201), arr: [301, 401, 501] },
    Row { name: "a4-config-row-2-with-a-longish-name", weight: 12, pair: (102, 202), arr: [302, 402, 502] },
    Row { name: "a4-config-row-3-with-a-longish-name", weight: 13, pair: (103, 203), arr: [303, 403, 503] },
    Row { name: "a4-config-row-4-with-a-longish-name", weight: 14, pair: (104, 204), arr: [304, 404, 504] },
    Row { name: "a4-config-row-5-with-a-longish-name", weight: 15, pair: (105, 205), arr: [305, 405, 505] }
], nums: &[8001, 8002, 8003, 8004, 8005, 8006, 8007, 8008, 8009, 8010, 8011, 8012, 8013, 8014, 8015, 8016, 8017, 8018, 8019, 8020, 8021, 8022, 8023, 8024, 8025, 8026, 8027, 8028, 8029, 8030, 8031, 8032, 8033, 8034, 8035, 8036, 8037, 8038, 8039, 8040, 8041, 8042, 8043, 8044, 8045, 8046, 8047, 8048, 8049, 8050, 8051, 8052, 8053, 8054, 8055, 8056, 8057, 8058, 8059, 8060, 8061, 8062, 8063, 8064, 8065, 8066, 8067, 8068, 8069, 8070, 8071, 8072, 8073, 8074, 8075, 8076, 8077, 8078, 8079, 8080], label: "a4-config", counts: [1, 2, 3, 4] };

// Associated consts and a const-fn-built table.
trait HasTable { const TABLE: &'static [u32]; fn total() -> u32 { Self::TABLE.iter().sum() } }
struct T1; struct T2; struct T3;
impl HasTable for T1 { const TABLE: &'static [u32] = &[9001, 9002, 9003, 9004, 9005, 9006, 9007, 9008, 9009, 9010, 9011, 9012, 9013, 9014, 9015, 9016, 9017, 9018, 9019, 9020, 9021, 9022, 9023, 9024, 9025, 9026, 9027, 9028, 9029, 9030, 9031, 9032, 9033, 9034, 9035, 9036, 9037, 9038, 9039, 9040, 9041, 9042, 9043, 9044, 9045, 9046, 9047, 9048, 9049, 9050, 9051, 9052, 9053, 9054, 9055, 9056, 9057, 9058, 9059, 9060, 9061, 9062, 9063, 9064, 9065, 9066, 9067, 9068, 9069, 9070]; }
impl HasTable for T2 { const TABLE: &'static [u32] = &[9001, 9002, 9003, 9004, 9005, 9006, 9007, 9008, 9009, 9010, 9011, 9012, 9013, 9014, 9015, 9016, 9017, 9018, 9019, 9020, 9021, 9022, 9023, 9024, 9025, 9026, 9027, 9028, 9029, 9030, 9031, 9032, 9033, 9034, 9035, 9036, 9037, 9038, 9039, 9040, 9041, 9042, 9043, 9044, 9045, 9046, 9047, 9048, 9049, 9050, 9051, 9052, 9053, 9054, 9055, 9056, 9057, 9058, 9059, 9060, 9061, 9062, 9063, 9064, 9065, 9066, 9067, 9068, 9069, 9070]; }
impl HasTable for T3 { const TABLE: &'static [u32] = &[9501, 9502, 9503, 9504, 9505, 9506, 9507, 9508, 9509, 9510, 9511, 9512, 9513, 9514, 9515, 9516, 9517, 9518, 9519, 9520, 9521, 9522, 9523, 9524, 9525, 9526, 9527, 9528, 9529, 9530, 9531, 9532, 9533, 9534, 9535, 9536, 9537, 9538, 9539, 9540, 9541, 9542, 9543, 9544, 9545, 9546, 9547, 9548, 9549, 9550, 9551, 9552, 9553, 9554, 9555, 9556, 9557, 9558, 9559, 9560, 9561, 9562, 9563, 9564, 9565, 9566, 9567, 9568, 9569, 9570]; }
const fn squares() -> [u32; 90] { let mut out = [0u32; 90]; let mut i = 0; while i < 90 { out[i] = (i * i) as u32 + 100; i += 1; } out }
const SQUARES: &[u32; 90] = &squares();
const SQ_BYVAL: [u32; 90] = squares();

// Identical print, different types.
const TUPS: &[(u32, u32)] = &[(0, 1000), (1, 1007), (2, 1014), (3, 1021), (4, 1028), (5, 1035), (6, 1042), (7, 1049), (8, 1056), (9, 1063), (10, 1070), (11, 1077), (12, 1084), (13, 1091), (14, 1098), (15, 1105), (16, 1112), (17, 1119), (18, 1126), (19, 1133), (20, 1140), (21, 1147), (22, 1154), (23, 1161), (24, 1168), (25, 1175), (26, 1182), (27, 1189), (28, 1196), (29, 1203), (30, 1210), (31, 1217), (32, 1224), (33, 1231), (34, 1238), (35, 1245), (36, 1252), (37, 1259), (38, 1266), (39, 1273)];
const ARRS: &[[u32; 2]] = &[[0, 1000], [1, 1007], [2, 1014], [3, 1021], [4, 1028], [5, 1035], [6, 1042], [7, 1049], [8, 1056], [9, 1063], [10, 1070], [11, 1077], [12, 1084], [13, 1091], [14, 1098], [15, 1105], [16, 1112], [17, 1119], [18, 1126], [19, 1133], [20, 1140], [21, 1147], [22, 1154], [23, 1161], [24, 1168], [25, 1175], [26, 1182], [27, 1189], [28, 1196], [29, 1203], [30, 1210], [31, 1217], [32, 1224], [33, 1231], [34, 1238], [35, 1245], [36, 1252], [37, 1259], [38, 1266], [39, 1273]];
#[derive(Clone, Copy)] struct P1 { x: u32, y: u32 }
#[derive(Clone, Copy)] struct P2 { x: u32, y: u32 }
const PS1: &[P1] = &[P1 { x: 0, y: 5000 }, P1 { x: 1, y: 5001 }, P1 { x: 2, y: 5002 }, P1 { x: 3, y: 5003 }, P1 { x: 4, y: 5004 }, P1 { x: 5, y: 5005 }, P1 { x: 6, y: 5006 }, P1 { x: 7, y: 5007 }, P1 { x: 8, y: 5008 }, P1 { x: 9, y: 5009 }, P1 { x: 10, y: 5010 }, P1 { x: 11, y: 5011 }, P1 { x: 12, y: 5012 }, P1 { x: 13, y: 5013 }, P1 { x: 14, y: 5014 }, P1 { x: 15, y: 5015 }, P1 { x: 16, y: 5016 }, P1 { x: 17, y: 5017 }, P1 { x: 18, y: 5018 }, P1 { x: 19, y: 5019 }, P1 { x: 20, y: 5020 }, P1 { x: 21, y: 5021 }, P1 { x: 22, y: 5022 }, P1 { x: 23, y: 5023 }];
const PS2: &[P2] = &[P2 { x: 0, y: 5000 }, P2 { x: 1, y: 5001 }, P2 { x: 2, y: 5002 }, P2 { x: 3, y: 5003 }, P2 { x: 4, y: 5004 }, P2 { x: 5, y: 5005 }, P2 { x: 6, y: 5006 }, P2 { x: 7, y: 5007 }, P2 { x: 8, y: 5008 }, P2 { x: 9, y: 5009 }, P2 { x: 10, y: 5010 }, P2 { x: 11, y: 5011 }, P2 { x: 12, y: 5012 }, P2 { x: 13, y: 5013 }, P2 { x: 14, y: 5014 }, P2 { x: 15, y: 5015 }, P2 { x: 16, y: 5016 }, P2 { x: 17, y: 5017 }, P2 { x: 18, y: 5018 }, P2 { x: 19, y: 5019 }, P2 { x: 20, y: 5020 }, P2 { x: 21, y: 5021 }, P2 { x: 22, y: 5022 }, P2 { x: 23, y: 5023 }];
#[derive(Clone, Copy)] enum E1 { U, V(u32) }
#[derive(Clone, Copy)] enum E2 { U, V(u32) }
const ES1: &[E1] = &[E1::V(6000), E1::V(6001), E1::V(6002), E1::V(6003), E1::V(6004), E1::V(6005), E1::V(6006), E1::V(6007), E1::V(6008), E1::V(6009), E1::V(6010), E1::V(6011), E1::V(6012), E1::V(6013), E1::V(6014), E1::V(6015), E1::V(6016), E1::V(6017), E1::V(6018), E1::V(6019), E1::V(6020), E1::V(6021), E1::V(6022), E1::V(6023), E1::V(6024), E1::V(6025), E1::V(6026), E1::V(6027), E1::V(6028), E1::V(6029)];
const ES2: &[E2] = &[E2::V(6000), E2::V(6001), E2::V(6002), E2::V(6003), E2::V(6004), E2::V(6005), E2::V(6006), E2::V(6007), E2::V(6008), E2::V(6009), E2::V(6010), E2::V(6011), E2::V(6012), E2::V(6013), E2::V(6014), E2::V(6015), E2::V(6016), E2::V(6017), E2::V(6018), E2::V(6019), E2::V(6020), E2::V(6021), E2::V(6022), E2::V(6023), E2::V(6024), E2::V(6025), E2::V(6026), E2::V(6027), E2::V(6028), E2::V(6029)];

#[inline(never)] fn statics() {
    print_usize(LONG_STATIC.len()); print_str(LONG_STATIC[7].1); print_str(FIRST_LONG.0); print_str(FIRST_LONG.1);
    print_usize(HOLDER.table.len()); print_str(HOLDER.table[2].1); print_u32(HOLDER.rows[1].weight); print_u32(HOLDER.grid[1][2].arr[0]); print_str(HOLDER.first.name);
    print_u32(CELLED.0.get()[4].weight); CELLED.0.set(&ROWS[2..]); print_u32(CELLED.0.get()[0].weight); print_usize(CELLED.0.get().len());
    print_u32(AAA_EARLY[7].weight); print_u32(ZZZ_LATE.0[6].weight); print_str(ZZZ_LATE.1[5].1);
    print_bool(core::ptr::eq(HOLDER.rows, HOLDER.grid[0]));
}
#[inline(never)] fn ops(x: &[u32]) {
    for (name, op) in OPS { print_str(name); print_u32(op(x)); }
    let mut total = 0u32; for (_, op) in OPS.iter().rev() { total = total.wrapping_mul(3).wrapping_add(op(x)); } print_u32(total);
    print_u32((OPS_STATIC[black_box(5)].1)(x)); print_str(OPS_STATIC[3].0);
}
#[inline(never)] fn config(round: u32) {
    let mut cfg = CFG; cfg.counts[0] += round; cfg.label = "changed"; cfg.rows = &cfg.rows[1..];
    print_u32(cfg.counts[0]); print_usize(cfg.rows.len()); print_u32(cfg.nums[79]); print_str(CFG.label); print_u32(CFG.counts[0]); print_usize(CFG.rows.len());
    let other = CFG; print_u32(other.rows[5].weight + other.nums[0]);
    let mut n = *CFG.nums; n[0] = 1; print_u32(CFG.nums[0]);
}
#[inline(never)] fn tables() {
    print_u32(T1::total()); print_u32(T2::total()); print_u32(T3::total()); print_u32(<T1 as HasTable>::TABLE[69] + T3::TABLE[0]);
    print_u32(SQUARES.iter().sum()); print_u32(SQUARES[89]); let mut sq = *SQUARES; sq[0] = 0; print_u32(SQUARES[0]);
    print_u32(SQ_BYVAL.iter().sum()); let mut sv = SQ_BYVAL; sv[1] = 0; print_u32(SQ_BYVAL[1]); print_u32(SQ_BYVAL.iter().rev().take(3).sum());
    print_u32(TUPS.iter().map(|t| t.0 + t.1).sum()); print_u32(ARRS.iter().map(|a| a[0] + a[1]).sum()); print_u32(TUPS[39].1 + ARRS[39][1]);
    print_u32(PS1.iter().map(|p| p.x + p.y).sum()); print_u32(PS2.iter().map(|p| p.x * 2 + p.y).sum()); print_u32(PS1[23].y + PS2[23].y);
    print_u32(ES1.iter().map(|e| match e { E1::U => 0, E1::V(x) => *x }).sum()); print_u32(ES2.iter().map(|e| match e { E2::U => 1, E2::V(x) => *x + 1 }).sum());
    print_usize(LONG.iter().map(|(k, v)| k.len() + v.len()).sum()); print_str(LONG[black_box(4)].1);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    for round in 0..2 { statics(); ops(&[5, 9, 2, 7]); config(round); tables(); }
}
