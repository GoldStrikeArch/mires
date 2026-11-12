
#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::future::Future;
use core::hint::black_box;
use core::pin::{pin, Pin};
use core::task::{Context, Poll, Waker};

#[derive(Clone, Copy)]
struct Row { name: &'static str, weight: u32, arr: [u32; 3] }
const ARR: &[u32; 100] = &[1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017, 1018, 1019, 1020, 1021, 1022, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037, 1038, 1039, 1040, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1049, 1050, 1051, 1052, 1053, 1054, 1055, 1056, 1057, 1058, 1059, 1060, 1061, 1062, 1063, 1064, 1065, 1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081, 1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1091, 1092, 1093, 1094, 1095, 1096, 1097, 1098, 1099, 1100];
const ROWS: &[Row] = &[
    Row { name: "a6-slice-row-0-with-a-longish-name", weight: 10, arr: [300, 400, 500] },
    Row { name: "a6-slice-row-1-with-a-longish-name", weight: 11, arr: [301, 401, 501] },
    Row { name: "a6-slice-row-2-with-a-longish-name", weight: 12, arr: [302, 402, 502] },
    Row { name: "a6-slice-row-3-with-a-longish-name", weight: 13, arr: [303, 403, 503] },
    Row { name: "a6-slice-row-4-with-a-longish-name", weight: 14, arr: [304, 404, 504] },
    Row { name: "a6-slice-row-5-with-a-longish-name", weight: 15, arr: [305, 405, 505] },
    Row { name: "a6-slice-row-6-with-a-longish-name", weight: 16, arr: [306, 406, 506] },
    Row { name: "a6-slice-row-7-with-a-longish-name", weight: 17, arr: [307, 407, 507] }
];
const ROWARR: &[Row; 8] = &[
    Row { name: "a6-array-row-0-with-a-longish-name", weight: 10, arr: [300, 400, 500] },
    Row { name: "a6-array-row-1-with-a-longish-name", weight: 11, arr: [301, 401, 501] },
    Row { name: "a6-array-row-2-with-a-longish-name", weight: 12, arr: [302, 402, 502] },
    Row { name: "a6-array-row-3-with-a-longish-name", weight: 13, arr: [303, 403, 503] },
    Row { name: "a6-array-row-4-with-a-longish-name", weight: 14, arr: [304, 404, 504] },
    Row { name: "a6-array-row-5-with-a-longish-name", weight: 15, arr: [305, 405, 505] },
    Row { name: "a6-array-row-6-with-a-longish-name", weight: 16, arr: [306, 406, 506] },
    Row { name: "a6-array-row-7-with-a-longish-name", weight: 17, arr: [307, 407, 507] }
];

struct PendOnce { polled: bool }
impl Future for PendOnce {
    type Output = u32;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
        if self.polled { return Poll::Ready(7); }
        self.polled = true;
        Poll::Pending
    }
}

/// Holds a pooled reference, a by-value copy of a pooled array and a copied row across awaits, and writes the copies.
async fn walk(seed: u32) -> u32 {
    let table = ROWS;
    let mut copy = *ARR;
    let mut rows = *ROWARR;
    let first = PendOnce { polled: false }.await;
    copy[0] += first + seed;
    rows[1].arr[0] += seed;
    let mut row = table[2];
    row.arr[0] += 1;
    let second = PendOnce { polled: false }.await;
    copy[1] = row.arr[0] + second;
    rows[1].weight = 0;
    let third = PendOnce { polled: false }.await;
    row.weight += third;
    table.iter().map(|r| r.weight).sum::<u32>() + copy[0] + copy[1] + ARR[0] + ARR[1] + ROWS[2].arr[0] + ROWARR[1].arr[0] + ROWARR[1].weight + rows[1].arr[0] + row.weight
}
async fn nested(seed: u32) -> u32 {
    let a = walk(seed).await;
    let held = &ROWS[seed as usize % 8];
    let b = walk(seed + 1).await;
    a + b + held.arr[1]
}
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop { if let Poll::Ready(value) = future.as_mut().poll(&mut context) { return value; } }
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(block_on(walk(1)));
    print_u32(block_on(walk(2)));
    print_u32(block_on(nested(3)));
    // Two coroutines alive at once, polled alternately.
    let mut f1 = pin!(walk(10));
    let mut f2 = pin!(nested(20));
    let mut context = Context::from_waker(Waker::noop());
    let (mut r1, mut r2) = (None, None);
    while r1.is_none() || r2.is_none() {
        if r1.is_none() { if let Poll::Ready(v) = f1.as_mut().poll(&mut context) { r1 = Some(v); } }
        if r2.is_none() { if let Poll::Ready(v) = f2.as_mut().poll(&mut context) { r2 = Some(v); } }
    }
    print_u32(r1.unwrap()); print_u32(r2.unwrap());
    print_u32(ARR[0] + ARR[1] + ROWS[2].arr[0] + ROWARR[1].arr[0] + ROWARR[1].weight);
}
