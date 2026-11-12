#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
#[derive(Clone, Copy)]
struct P { x: u32, y: u32 }
const LOCAL_SAME: &[u32] = &[7001, 7002, 7003, 7004, 7005, 7006, 7007, 7008, 7009, 7010, 7011, 7012, 7013, 7014, 7015, 7016, 7017, 7018, 7019, 7020, 7021, 7022, 7023, 7024, 7025, 7026, 7027, 7028, 7029, 7030, 7031, 7032, 7033, 7034, 7035, 7036, 7037, 7038, 7039, 7040, 7041, 7042, 7043, 7044, 7045, 7046, 7047, 7048, 7049, 7050, 7051, 7052, 7053, 7054, 7055, 7056, 7057, 7058, 7059, 7060, 7061, 7062, 7063, 7064, 7065, 7066, 7067, 7068, 7069, 7070, 7071, 7072, 7073, 7074, 7075, 7076, 7077, 7078, 7079, 7080, 7081, 7082, 7083, 7084, 7085, 7086, 7087, 7088, 7089, 7090, 7091, 7092, 7093, 7094, 7095, 7096, 7097, 7098, 7099, 7100];
const PS: &[P] = &[P { x: 0, y: 5000 }, P { x: 1, y: 5001 }, P { x: 2, y: 5002 }, P { x: 3, y: 5003 }, P { x: 4, y: 5004 }, P { x: 5, y: 5005 }, P { x: 6, y: 5006 }, P { x: 7, y: 5007 }, P { x: 8, y: 5008 }, P { x: 9, y: 5009 }, P { x: 10, y: 5010 }, P { x: 11, y: 5011 }, P { x: 12, y: 5012 }, P { x: 13, y: 5013 }, P { x: 14, y: 5014 }, P { x: 15, y: 5015 }, P { x: 16, y: 5016 }, P { x: 17, y: 5017 }, P { x: 18, y: 5018 }, P { x: 19, y: 5019 }, P { x: 20, y: 5020 }, P { x: 21, y: 5021 }, P { x: 22, y: 5022 }, P { x: 23, y: 5023 }];
#[inline(never)] fn local_sum() -> u32 { LOCAL_SAME.iter().sum::<u32>() + PS.iter().map(|p| p.x + p.y).sum::<u32>() }
#[inline(never)] fn cross_sum() -> u32 { a5dep1::SHARED_TABLE.iter().sum::<u32>() + a5dep2::SHARED_TABLE[5] + a5dep1::PS[3].y + a5dep2::PS.iter().map(|p| p.x).sum::<u32>() + a5dep1::DEP_ONLY[1] + a5dep2::DEP_ONLY[1] }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    for _ in 0..2 {
        print_u32(local_sum()); print_u32(cross_sum());
        print_u32(a5dep1::dep_sum()); print_u32(a5dep2::dep_sum()); print_u32(a5dep1::dep_first()); print_u32(a5dep2::dep_first());
        print_u32(a5dep1::dep_copy_and_write()); print_u32(a5dep2::dep_copy_and_write());
        let mut p = PS[4]; p.x = 99; let mut d = a5dep1::PS[4]; d.x = 98; print_u32(PS[4].x + a5dep1::PS[4].x + a5dep2::PS[4].x);
        print_u32(LOCAL_SAME[0] + a5dep1::SHARED_TABLE[0] + a5dep2::SHARED_TABLE[0]);
    }
}
