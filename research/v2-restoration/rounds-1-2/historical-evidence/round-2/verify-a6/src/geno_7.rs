#![no_std]
#![no_main]
#![allow(dead_code, unused, non_upper_case_globals)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};
enum E0 { None, One(&'static T0), Two(u32, &'static T0, &'static T0) }
struct T0 { id: u32, hits: AtomicU32, same: &'static [&'static T0], heads: &'static [&'static &'static T0], one: &'static T0, down: &'static [&'static T1], o: Option<&'static T0>, e: E0 }
enum E1 { None, One(&'static T1), Two(u32, &'static T1, &'static T1) }
struct T1 { id: u32, hits: AtomicU32, same: &'static [&'static T1], heads: &'static [&'static &'static T1], down: &'static [&'static T2], up: &'static [&'static T0], o: Option<&'static T1>, e: E1 }
enum E2 { None, One(&'static T2), Two(u32, &'static T2, &'static T2) }
struct T2 { id: u32, hits: AtomicU32, same: &'static [&'static T2], heads: &'static [&'static &'static T2], pair: [&'static T2; 2], up: &'static [&'static T1], o: Option<&'static T2>, e: E2 }
static N1_8: T1 = T1 { id: 15, hits: AtomicU32::new(0), same: &[], heads: &[&H1_2, &H1_2], down: &[&N2_7], up: &[], o: Some(&N1_5), e: E1::None };
static N1_2: T1 = T1 { id: 9, hits: AtomicU32::new(0), same: &[], heads: &[], down: &[], up: &[], o: Some(&N1_0), e: E1::Two(4, &N1_4, &N1_5) };
static N2_7: T2 = T2 { id: 23, hits: AtomicU32::new(0), same: &[&N2_7, &N2_0, &N2_1], heads: &[], pair: [&N2_2, &N2_2], up: &[], o: None, e: E2::Two(7, &N2_8, &N2_8) };
static N1_3: T1 = T1 { id: 10, hits: AtomicU32::new(0), same: &[&N1_7], heads: &[], down: &[], up: &[&N0_1], o: None, e: E1::None };
static N1_6: T1 = T1 { id: 13, hits: AtomicU32::new(0), same: &[&N1_6, &N1_0, &N1_2], heads: &[], down: &[&N2_4], up: &[], o: Some(&N1_0), e: E1::None };
static N0_4: T0 = T0 { id: 5, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_2, down: &[], o: Some(&N0_0), e: E0::Two(5, &N0_1, &N0_1) };
static N0_0: T0 = T0 { id: 1, hits: AtomicU32::new(0), same: &[], heads: &[&H0_1], one: &N0_4, down: &[], o: Some(&N0_2), e: E0::Two(4, &N0_1, &N0_4) };
static N0_5: T0 = T0 { id: 6, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_0, down: &[], o: Some(&N0_3), e: E0::Two(4, &N0_5, &N0_2) };
static N1_0: T1 = T1 { id: 7, hits: AtomicU32::new(0), same: &[&N1_5, &N1_0, &N1_6], heads: &[&H1_2, &H1_0], down: &[&N2_6], up: &[], o: Some(&N1_5), e: E1::Two(1, &N1_3, &N1_6) };
static N2_0: T2 = T2 { id: 16, hits: AtomicU32::new(0), same: &[], heads: &[], pair: [&N2_6, &N2_0], up: &[], o: Some(&N2_0), e: E2::None };
static N2_2: T2 = T2 { id: 18, hits: AtomicU32::new(0), same: &[&N2_8, &N2_2, &N2_3], heads: &[], pair: [&N2_6, &N2_2], up: &[], o: Some(&N2_7), e: E2::None };
static N2_4: T2 = T2 { id: 20, hits: AtomicU32::new(0), same: &[&N2_6, &N2_8, &N2_6], heads: &[&H2_1, &H2_0], pair: [&N2_3, &N2_1], up: &[], o: None, e: E2::Two(1, &N2_5, &N2_1) };
static H1_1: &T1 = &N1_7;
static N1_7: T1 = T1 { id: 14, hits: AtomicU32::new(0), same: &[&N1_2], heads: &[], down: &[&N2_2, &N2_3], up: &[], o: Some(&N1_5), e: E1::None };
static N2_6: T2 = T2 { id: 22, hits: AtomicU32::new(0), same: &[&N2_1, &N2_7], heads: &[&H2_0, &H2_1], pair: [&N2_6, &N2_2], up: &[], o: None, e: E2::Two(3, &N2_5, &N2_7) };
static H1_0: &T1 = &N1_7;
static N1_5: T1 = T1 { id: 12, hits: AtomicU32::new(0), same: &[], heads: &[], down: &[], up: &[], o: None, e: E1::One(&N1_2) };
static N2_5: T2 = T2 { id: 21, hits: AtomicU32::new(0), same: &[], heads: &[], pair: [&N2_6, &N2_2], up: &[], o: None, e: E2::One(&N2_6) };
static N2_3: T2 = T2 { id: 19, hits: AtomicU32::new(0), same: &[], heads: &[], pair: [&N2_6, &N2_3], up: &[], o: Some(&N2_1), e: E2::None };
static H0_1: &T0 = &N0_2;
static N0_3: T0 = T0 { id: 4, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_2, down: &[&N1_6], o: Some(&N0_1), e: E0::None };
static N2_8: T2 = T2 { id: 24, hits: AtomicU32::new(0), same: &[&N2_6, &N2_1], heads: &[&H2_1], pair: [&N2_5, &N2_6], up: &[], o: Some(&N2_4), e: E2::Two(3, &N2_7, &N2_3) };
static N1_4: T1 = T1 { id: 11, hits: AtomicU32::new(0), same: &[], heads: &[], down: &[], up: &[], o: Some(&N1_2), e: E1::One(&N1_3) };
static H0_2: &T0 = &N0_1;
static H2_1: &T2 = &N2_3;
static N2_1: T2 = T2 { id: 17, hits: AtomicU32::new(0), same: &[&N2_4], heads: &[], pair: [&N2_1, &N2_1], up: &[&N1_4], o: None, e: E2::None };
static H1_2: &T1 = &N1_8;
static H0_0: &T0 = &N0_3;
static N0_2: T0 = T0 { id: 3, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_1, down: &[], o: None, e: E0::None };
static N0_1: T0 = T0 { id: 2, hits: AtomicU32::new(0), same: &[], heads: &[], one: &N0_1, down: &[], o: Some(&N0_1), e: E0::None };
static N1_1: T1 = T1 { id: 8, hits: AtomicU32::new(0), same: &[&N1_4, &N1_6], heads: &[&H1_2], down: &[&N2_5, &N2_8], up: &[&N0_3], o: Some(&N1_6), e: E1::Two(7, &N1_6, &N1_1) };
static H2_0: &T2 = &N2_3;
fn walk0(n: &'static T0, depth: u32) -> u32 {
    n.hits.fetch_add(1, Ordering::Relaxed);
    if depth == 0 { return n.id; }
    let mut t = n.id.wrapping_mul(31);
    for s in n.same.iter() { t = t.wrapping_mul(7).wrapping_add(walk0(s, depth - 1)); }
    for h in n.heads.iter() { t = t.wrapping_mul(11).wrapping_add(walk0(**h, depth - 1)); }
    t = t.wrapping_mul(13).wrapping_add(walk0(n.one, depth - 1));
    for d in n.down.iter() { t = t.wrapping_mul(19).wrapping_add(walk1(d, depth - 1)); }
    if let Some(o) = n.o { t = t.wrapping_mul(29).wrapping_add(walk0(o, depth - 1)); } else { t ^= 0x55; }
    match &n.e { E0::None => t ^= 0xAA, E0::One(a) => t = t.wrapping_mul(37).wrapping_add(walk0(a, depth - 1)), E0::Two(x, a, b) => t = t.wrapping_mul(41).wrapping_add(*x).wrapping_add(walk0(a, depth - 1)).wrapping_sub(walk0(b, depth - 1)) }
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
    if let Some(o) = n.o { t = t.wrapping_mul(29).wrapping_add(walk1(o, depth - 1)); } else { t ^= 0x55; }
    match &n.e { E1::None => t ^= 0xAA, E1::One(a) => t = t.wrapping_mul(37).wrapping_add(walk1(a, depth - 1)), E1::Two(x, a, b) => t = t.wrapping_mul(41).wrapping_add(*x).wrapping_add(walk1(a, depth - 1)).wrapping_sub(walk1(b, depth - 1)) }
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
    if let Some(o) = n.o { t = t.wrapping_mul(29).wrapping_add(walk2(o, depth - 1)); } else { t ^= 0x55; }
    match &n.e { E2::None => t ^= 0xAA, E2::One(a) => t = t.wrapping_mul(37).wrapping_add(walk2(a, depth - 1)), E2::Two(x, a, b) => t = t.wrapping_mul(41).wrapping_add(*x).wrapping_add(walk2(a, depth - 1)).wrapping_sub(walk2(b, depth - 1)) }
    t
}
#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("a");
    print_u32(walk2(&N2_6, 4));
    print_u32(walk1(&N1_3, 4));
    print_u32(walk1(&N1_6, 4));
    print_u32(walk2(&N2_3, 4));
    print_u32(walk0(&N0_3, 4));
    print_u32(walk1(&N1_1, 4));
    print_u32(walk0(&N0_5, 4));
    print_u32(N2_3.hits.load(Ordering::Relaxed));
    print_u32(N2_1.hits.load(Ordering::Relaxed));
    print_u32(N1_8.hits.load(Ordering::Relaxed));
    print_u32(N0_4.hits.load(Ordering::Relaxed));
    print_u32(N2_4.hits.load(Ordering::Relaxed));
    print_bool(N0_1.same.len() > 0 && core::ptr::eq(N0_1.same[0], &N0_0));
    print_bool(core::ptr::eq(N0_1.one, &N0_0) || core::ptr::eq(N0_1.one.one, &N0_1));
    print_bool(N1_4.same.len() > 0 && core::ptr::eq(N1_4.same[0], &N1_3));
    print_bool(N2_2.same.len() > 0 && core::ptr::eq(N2_2.same[0], &N2_7));
    print_bool(core::ptr::eq(N2_2.pair[1], &N2_2) || core::ptr::eq(N2_2.pair[0], &N2_7));
}
#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("b");
    print_u32(walk1(&N1_4, 4));
    print_u32(walk0(&N0_4, 4));
    print_u32(walk1(&N1_6, 4));
    print_u32(walk2(&N2_4, 4));
    print_u32(walk2(&N2_7, 4));
    print_u32(walk1(&N1_7, 4));
    print_u32(N1_4.hits.load(Ordering::Relaxed));
    print_u32(N1_6.hits.load(Ordering::Relaxed));
    print_u32(N2_0.hits.load(Ordering::Relaxed));
    print_u32(N1_0.hits.load(Ordering::Relaxed));
    print_u32(N1_2.hits.load(Ordering::Relaxed));
    print_bool(N0_4.same.len() > 0 && core::ptr::eq(N0_4.same[0], &N0_3));
    print_bool(core::ptr::eq(N0_4.one, &N0_3) || core::ptr::eq(N0_4.one.one, &N0_4));
    print_bool(N1_0.same.len() > 0 && core::ptr::eq(N1_0.same[0], &N1_3));
    print_bool(N2_5.same.len() > 0 && core::ptr::eq(N2_5.same[0], &N2_2));
    print_bool(core::ptr::eq(N2_5.pair[1], &N2_5) || core::ptr::eq(N2_5.pair[0], &N2_2));
}
#[unsafe(no_mangle)]
pub fn chunk_c() {
    print_str("c");
    print_u32(walk2(&N2_2, 4));
    print_u32(walk1(&N1_3, 4));
    print_u32(walk2(&N2_7, 4));
    print_u32(walk1(&N1_2, 4));
    print_u32(N1_4.hits.load(Ordering::Relaxed));
    print_u32(N2_7.hits.load(Ordering::Relaxed));
    print_u32(N0_0.hits.load(Ordering::Relaxed));
    print_u32(N2_4.hits.load(Ordering::Relaxed));
    print_u32(N0_1.hits.load(Ordering::Relaxed));
    print_bool(N0_0.same.len() > 0 && core::ptr::eq(N0_0.same[0], &N0_1));
    print_bool(core::ptr::eq(N0_0.one, &N0_1) || core::ptr::eq(N0_0.one.one, &N0_0));
    print_bool(N1_3.same.len() > 0 && core::ptr::eq(N1_3.same[0], &N1_4));
    print_bool(N2_4.same.len() > 0 && core::ptr::eq(N2_4.same[0], &N2_7));
    print_bool(core::ptr::eq(N2_4.pair[1], &N2_4) || core::ptr::eq(N2_4.pair[0], &N2_7));
}
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); chunk_c(); }
