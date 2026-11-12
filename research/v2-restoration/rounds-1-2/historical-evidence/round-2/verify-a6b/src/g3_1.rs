#![no_std]
#![no_main]
#![allow(dead_code, unused, non_upper_case_globals, private_interfaces)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};
static ZERO: AtomicU32 = AtomicU32::new(0);
trait Sup { fn s(&self) -> u32; }
trait Tr: Sup + Sync { fn v(&self) -> u32; fn w(&self) -> u32 { self.v().wrapping_mul(3).wrapping_add(1) } }
struct Node { id: u32, kids: &'static [&'static Node], f: fn() -> u32, g: &'static [fn()], s: &'static str }
struct Imp0(u32);
impl Sup for Imp0 { fn s(&self) -> u32 { self.0.wrapping_add(S_3.id) } }
impl Tr for Imp0 { fn v(&self) -> u32 { self.0.wrapping_add(S_0.id) } }
static D_0: Imp0 = Imp0(5);
struct Imp1(u32);
impl Sup for Imp1 { fn s(&self) -> u32 { self.0.wrapping_add(S_3.id) } }
impl Tr for Imp1 { fn v(&self) -> u32 { self.0 } }
static D_1: Imp1 = Imp1(6);
#[unsafe(no_mangle)]
pub static S_3: Node = Node { id: 4, kids: &[], f: h_2, g: &[ent_2], s: "three long strings make a pool of hoisted constants ok" };
#[unsafe(no_mangle)]
pub static S_1: Node = Node { id: 2, kids: &[], f: h_1, g: &[], s: "three long strings make a pool of hoisted constants ok" };
static S_0: Node = Node { id: 1, kids: &[&S_3], f: h_1, g: &[ent_0], s: "three long strings make a pool of hoisted constants ok" };
#[unsafe(no_mangle)]
pub static S_2: Node = Node { id: 3, kids: &[], f: h_0, g: &[], s: "a second long string literal that the compiler may hoist" };
fn walk(n: &'static Node, depth: u32) -> u32 {
    let mut t = n.id.wrapping_mul(31).wrapping_add((n.f)()).wrapping_add(n.s.len() as u32);
    if n.g.len() > 0 { t = t.wrapping_add(1000); }
    if depth > 0 { for k in n.kids.iter() { t = t.wrapping_mul(7).wrapping_add(walk(k, depth - 1)); } }
    t
}
#[inline(never)]
fn dynv(d: &dyn Tr) -> u32 { let s: &dyn Sup = d; d.v().wrapping_add(d.w()).wrapping_add(s.s()) }
#[inline(never)]
fn h_0() -> u32 {
    S_3.id.wrapping_add(dynv(&D_1).wrapping_add(Some(S_3.id).filter(|v| *v < 1000).expect("an id below one thousand, as every generated id is")))
}
#[inline(never)]
fn h_1() -> u32 {
    S_3.id.wrapping_add(("the quick brown fox jumps over the lazy dog, twice over".len() as u32).wrapping_add(dynv(&D_1).wrapping_add(Some(S_1.id).filter(|v| *v < 1000).expect("an id below one thousand, as every generated id is"))))
}
#[inline(never)]
fn h_2() -> u32 {
    S_2.id.wrapping_add(("three long strings make a pool of hoisted constants ok".len() as u32).wrapping_add(dynv(&D_0).wrapping_add(Some(S_3.id).filter(|v| *v < 1000).expect("an id below one thousand, as every generated id is"))))
}
#[unsafe(no_mangle)]
pub fn ent_0() {
    print_u32(walk(&S_0, 3));
    print_u32(walk(&S_3, 3));
    print_u32(walk(&S_0, 3));
    print_u32(h_2());
    ent_1();
    if ZERO.load(Ordering::Relaxed) != 0 { ent_1(); }
    print_bool(S_0.kids.len() > 0 && core::ptr::eq(S_0.kids[0], &S_1));
}
#[unsafe(no_mangle)]
pub fn ent_1() {
    print_u32(walk(&S_1, 3));
    print_u32(walk(&S_3, 3));
    print_u32(walk(&S_2, 3));
    ent_2();
    print_bool(S_0.kids.len() > 0 && core::ptr::eq(S_0.kids[0], &S_3));
}
#[unsafe(no_mangle)]
pub fn ent_2() {
    print_u32(walk(&S_1, 3));
    print_u32(walk(&S_1, 3));
    print_u32(walk(&S_3, 3));
    print_bool(S_1.kids.len() > 0 && core::ptr::eq(S_1.kids[0], &S_3));
    print_str(S_2.s);
}
#[unsafe(no_mangle)]
pub fn rust_entry() { print_str("== 0"); ent_0(); print_str("== 1"); ent_1(); print_str("== 2"); ent_2(); }
