#![no_std]
#![no_main]
#![allow(dead_code, unused, non_upper_case_globals)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};
struct T0 { id: u32, hits: AtomicU32, same: &'static [&'static T0], heads: &'static [&'static &'static T0], one: &'static T0, down: &'static [&'static T1] }
struct T1 { id: u32, hits: AtomicU32, same: &'static [&'static T1], heads: &'static [&'static &'static T1], down: &'static [&'static T2], up: &'static [&'static T0] }
struct T2 { id: u32, hits: AtomicU32, same: &'static [&'static T2], heads: &'static [&'static &'static T2], pair: [&'static T2; 2], up: &'static [&'static T1] }
static N0_1: T0 = T0 { id: 2, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_6, down: &[] };
static N1_3: T1 = T1 { id: 13, hits: AtomicU32::new(0), same: &[], heads: &[], down: &[], up: &[] };
static N2_1: T2 = T2 { id: 19, hits: AtomicU32::new(0), same: &[&N2_4, &N2_3, &N2_6], heads: &[&H2_0], pair: [&N2_4, &N2_5], up: &[] };
static H1_1: &T1 = &N1_4;
static N1_6: T1 = T1 { id: 16, hits: AtomicU32::new(0), same: &[&N1_5, &N1_5], heads: &[], down: &[], up: &[] };
static N0_0: T0 = T0 { id: 1, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_0, down: &[] };
static H2_0: &T2 = &N2_1;
static N0_5: T0 = T0 { id: 6, hits: AtomicU32::new(0), same: &[&N0_2, &N0_6, &N0_8], heads: &[&H0_0], one: &N0_6, down: &[&N1_0] };
static N1_0: T1 = T1 { id: 10, hits: AtomicU32::new(0), same: &[&N1_5, &N1_2], heads: &[&H1_0], down: &[&N2_4, &N2_6], up: &[] };
static H1_0: &T1 = &N1_2;
static N1_7: T1 = T1 { id: 17, hits: AtomicU32::new(0), same: &[&N1_4, &N1_6], heads: &[&H1_0], down: &[&N2_0, &N2_4], up: &[] };
static N2_5: T2 = T2 { id: 23, hits: AtomicU32::new(0), same: &[], heads: &[], pair: [&N2_2, &N2_0], up: &[] };
static N2_0: T2 = T2 { id: 18, hits: AtomicU32::new(0), same: &[], heads: &[], pair: [&N2_2, &N2_6], up: &[] };
static N2_4: T2 = T2 { id: 22, hits: AtomicU32::new(0), same: &[&N2_4, &N2_1], heads: &[], pair: [&N2_4, &N2_1], up: &[] };
static N0_2: T0 = T0 { id: 3, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_8, down: &[] };
static H0_1: &T0 = &N0_4;
static N0_4: T0 = T0 { id: 5, hits: AtomicU32::new(0), same: &[&N0_5], heads: &[&H0_0, &H0_0], one: &N0_4, down: &[&N1_1] };
static N1_5: T1 = T1 { id: 15, hits: AtomicU32::new(0), same: &[&N1_2, &N1_3, &N1_4], heads: &[], down: &[&N2_0, &N2_1], up: &[] };
static N0_8: T0 = T0 { id: 9, hits: AtomicU32::new(0), same: &[&N0_0, &N0_4], heads: &[], one: &N0_4, down: &[] };
static N2_3: T2 = T2 { id: 21, hits: AtomicU32::new(0), same: &[], heads: &[], pair: [&N2_5, &N2_4], up: &[] };
static N2_2: T2 = T2 { id: 20, hits: AtomicU32::new(0), same: &[&N2_1], heads: &[&H2_0], pair: [&N2_6, &N2_2], up: &[] };
static N0_3: T0 = T0 { id: 4, hits: AtomicU32::new(0), same: &[&N0_2, &N0_8, &N0_0], heads: &[], one: &N0_3, down: &[] };
static N0_6: T0 = T0 { id: 7, hits: AtomicU32::new(0), same: &[&N0_0, &N0_7], heads: &[&H0_0], one: &N0_6, down: &[] };
static N0_7: T0 = T0 { id: 8, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_3, down: &[] };
static H0_0: &T0 = &N0_8;
static N1_1: T1 = T1 { id: 11, hits: AtomicU32::new(0), same: &[], heads: &[], down: &[], up: &[] };
static N1_2: T1 = T1 { id: 12, hits: AtomicU32::new(0), same: &[], heads: &[], down: &[], up: &[] };
static N2_6: T2 = T2 { id: 24, hits: AtomicU32::new(0), same: &[&N2_4], heads: &[], pair: [&N2_3, &N2_6], up: &[] };
static N1_4: T1 = T1 { id: 14, hits: AtomicU32::new(0), same: &[&N1_4, &N1_1], heads: &[&H1_1, &H1_0], down: &[&N2_4, &N2_4], up: &[] };
fn walk0(n: &'static T0, depth: u32) -> u32 {
    n.hits.fetch_add(1, Ordering::Relaxed);
    if depth == 0 { return n.id; }
    let mut t = n.id.wrapping_mul(31);
    for s in n.same.iter() { t = t.wrapping_mul(7).wrapping_add(walk0(s, depth - 1)); }
    for h in n.heads.iter() { t = t.wrapping_mul(11).wrapping_add(walk0(**h, depth - 1)); }
    t = t.wrapping_mul(13).wrapping_add(walk0(n.one, depth - 1));
    for d in n.down.iter() { t = t.wrapping_mul(19).wrapping_add(walk1(d, depth - 1)); }
    t
}
fn walk1(n: &'static T1, depth: u32) -> u32 {
    n.hits.fetch_add(1, Ordering::Relaxed);
    if depth == 0 { return n.id; }
    let mut t = n.id.wrapping_mul(31);
    for s in n.same.iter() { t = t.wrapping_mul(7).wrapping_add(walk1(s, depth - 1)); }
    for h in n.heads.iter() { t = t.wrapping_mul(11).wrapping_add(walk1(**h, depth - 1)); }
    for d in n.down.iter() { t = t.wrapping_mul(19).wrapping_add(walk2(d, depth - 1)); }
    for u in n.up.iter() { t = t.wrapping_mul(23).wrapping_add(walk0(u, depth - 1)); }
    t
}
fn walk2(n: &'static T2, depth: u32) -> u32 {
    n.hits.fetch_add(1, Ordering::Relaxed);
    if depth == 0 { return n.id; }
    let mut t = n.id.wrapping_mul(31);
    for s in n.same.iter() { t = t.wrapping_mul(7).wrapping_add(walk2(s, depth - 1)); }
    for h in n.heads.iter() { t = t.wrapping_mul(11).wrapping_add(walk2(**h, depth - 1)); }
    for p in n.pair.iter() { t = t.wrapping_mul(17).wrapping_add(walk2(p, depth - 1)); }
    for u in n.up.iter() { t = t.wrapping_mul(23).wrapping_add(walk1(u, depth - 1)); }
    t
}
#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("a");
    print_u32(walk2(&N2_0, 4));
    print_u32(walk0(&N0_7, 4));
    print_u32(walk2(&N2_3, 4));
    print_u32(walk0(&N0_0, 4));
    print_u32(walk1(&N1_3, 4));
    print_u32(walk0(&N0_3, 4));
    print_u32(N1_4.hits.load(Ordering::Relaxed));
    print_u32(N0_1.hits.load(Ordering::Relaxed));
    print_u32(N2_6.hits.load(Ordering::Relaxed));
    print_u32(N2_0.hits.load(Ordering::Relaxed));
    print_u32(N0_6.hits.load(Ordering::Relaxed));
    print_bool(N0_8.same.len() > 0 && core::ptr::eq(N0_8.same[0], &N0_4));
    print_bool(core::ptr::eq(N0_8.one, &N0_4) || core::ptr::eq(N0_8.one.one, &N0_8));
    print_bool(N1_5.same.len() > 0 && core::ptr::eq(N1_5.same[0], &N1_5));
    print_bool(N2_3.same.len() > 0 && core::ptr::eq(N2_3.same[0], &N2_0));
    print_bool(core::ptr::eq(N2_3.pair[1], &N2_3) || core::ptr::eq(N2_3.pair[0], &N2_0));
}
#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("b");
    print_u32(walk2(&N2_6, 4));
    print_u32(walk0(&N0_1, 4));
    print_u32(walk2(&N2_3, 4));
    print_u32(N2_6.hits.load(Ordering::Relaxed));
    print_u32(N1_0.hits.load(Ordering::Relaxed));
    print_u32(N1_1.hits.load(Ordering::Relaxed));
    print_u32(N0_3.hits.load(Ordering::Relaxed));
    print_u32(N0_4.hits.load(Ordering::Relaxed));
    print_bool(N0_8.same.len() > 0 && core::ptr::eq(N0_8.same[0], &N0_3));
    print_bool(core::ptr::eq(N0_8.one, &N0_3) || core::ptr::eq(N0_8.one.one, &N0_8));
    print_bool(N1_4.same.len() > 0 && core::ptr::eq(N1_4.same[0], &N1_7));
    print_bool(N2_2.same.len() > 0 && core::ptr::eq(N2_2.same[0], &N2_1));
    print_bool(core::ptr::eq(N2_2.pair[1], &N2_2) || core::ptr::eq(N2_2.pair[0], &N2_1));
}
#[unsafe(no_mangle)]
pub fn chunk_c() {
    print_str("c");
    print_u32(walk1(&N1_0, 4));
    print_u32(walk2(&N2_4, 4));
    print_u32(walk0(&N0_6, 4));
    print_u32(walk1(&N1_6, 4));
    print_u32(walk1(&N1_2, 4));
    print_u32(walk0(&N0_4, 4));
    print_u32(N0_6.hits.load(Ordering::Relaxed));
    print_u32(N1_7.hits.load(Ordering::Relaxed));
    print_u32(N2_1.hits.load(Ordering::Relaxed));
    print_u32(N0_4.hits.load(Ordering::Relaxed));
    print_u32(N1_0.hits.load(Ordering::Relaxed));
    print_bool(N0_1.same.len() > 0 && core::ptr::eq(N0_1.same[0], &N0_6));
    print_bool(core::ptr::eq(N0_1.one, &N0_6) || core::ptr::eq(N0_1.one.one, &N0_1));
    print_bool(N1_6.same.len() > 0 && core::ptr::eq(N1_6.same[0], &N1_2));
    print_bool(N2_1.same.len() > 0 && core::ptr::eq(N2_1.same[0], &N2_4));
    print_bool(core::ptr::eq(N2_1.pair[1], &N2_1) || core::ptr::eq(N2_1.pair[0], &N2_4));
}
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); chunk_c(); }
