#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};

// Statics whose texts are identical up to their own names: they must stay distinct objects.
struct Node { id: u32, hits: AtomicU32, next: &'static Node }
static A: Node = Node { id: 1, hits: AtomicU32::new(0), next: &B };
static B: Node = Node { id: 1, hits: AtomicU32::new(0), next: &A };
static S1: Node = Node { id: 7, hits: AtomicU32::new(0), next: &S1 };
static S2: Node = Node { id: 7, hits: AtomicU32::new(0), next: &S2 };
// two disjoint rings of the same shape
static P1: Node = Node { id: 3, hits: AtomicU32::new(0), next: &P2 };
static P2: Node = Node { id: 4, hits: AtomicU32::new(0), next: &P1 };
static Q1: Node = Node { id: 3, hits: AtomicU32::new(0), next: &Q2 };
static Q2: Node = Node { id: 4, hits: AtomicU32::new(0), next: &Q1 };
// diamond into a cycle, a cycle into a diamond, and a static reached only through other statics
struct D { id: u32, l: &'static D2, r: &'static D2 }
struct D2 { id: u32, c: &'static Node, bottom: &'static Bottom }
struct Bottom { id: u32 }
static BOT: Bottom = Bottom { id: 9 };
static TOP: D = D { id: 1, l: &LEFT, r: &RIGHT };
static LEFT: D2 = D2 { id: 2, c: &P1, bottom: &BOT };
static RIGHT: D2 = D2 { id: 3, c: &Q2, bottom: &BOT };
struct CD { id: u32, top: &'static D, back: &'static CD, bottom: &'static Bottom }
static CD1: CD = CD { id: 5, top: &TOP, back: &CD2, bottom: &BOT };
static CD2: CD = CD { id: 6, top: &TOP, back: &CD1, bottom: &BOT };
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_bool(core::ptr::eq(&A, &B));
    print_bool(core::ptr::eq(A.next, &B) && core::ptr::eq(B.next, &A));
    A.next.hits.fetch_add(5, Ordering::Relaxed);
    print_u32(A.hits.load(Ordering::Relaxed) * 10 + B.hits.load(Ordering::Relaxed));
    print_bool(core::ptr::eq(&S1, &S2));
    print_bool(core::ptr::eq(S1.next, &S1) && core::ptr::eq(S2.next.next, &S2));
    S1.next.hits.fetch_add(3, Ordering::Relaxed);
    print_u32(S1.hits.load(Ordering::Relaxed) * 10 + S2.hits.load(Ordering::Relaxed));
    print_bool(core::ptr::eq(P1.next, &Q2) || core::ptr::eq(&P1, &Q1));
    P1.next.next.hits.fetch_add(2, Ordering::Relaxed);
    Q2.next.next.hits.fetch_add(4, Ordering::Relaxed);
    print_u32(P1.hits.load(Ordering::Relaxed) * 1000 + P2.hits.load(Ordering::Relaxed) * 100 + Q1.hits.load(Ordering::Relaxed) * 10 + Q2.hits.load(Ordering::Relaxed));
    print_u32(TOP.l.c.next.id * 10 + TOP.r.c.next.id + TOP.l.bottom.id * 100);
    print_bool(core::ptr::eq(TOP.l.bottom, TOP.r.bottom));
    print_bool(core::ptr::eq(TOP.l.c, &P1) && core::ptr::eq(TOP.r.c.next, &Q1));
    print_u32(CD1.back.top.r.id + CD2.back.back.top.l.c.id * 10 + CD1.bottom.id * 100);
    print_bool(core::ptr::eq(CD1.top, CD2.top) && core::ptr::eq(CD1.back.back, &CD1) && core::ptr::eq(CD2.bottom, TOP.l.bottom));
}
