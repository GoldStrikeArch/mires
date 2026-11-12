#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};

// Five chunks. Ownership shapes:
//  * SH1/SH2: a cycle reached by a and b (shared), mutated in a, read in b
//  * AO1/AO2/AO3: a ring only chunk a reaches; AO3 names the shared LEAF and chunk c's entry
//  * A_TO_SH: a-owned, points into the shared cycle; A_SELF a-owned self reference
//  * B_TAB: b-owned array naming shared statics and b-owned BO (which names chunk a's entry)
//  * CDE1/CDE2: cycle reached by c, d and e (shared); CDE2 names chunk e's entry (shared imports e)
//  * chunk d calls chunk e (island to island): everything e reaches is shared, e owns its entry
//  * E_ONLY ring is reached by e and (through e) by d: shared
//  * SHT: shared table of fn pointers naming three entries
struct Node { id: u32, hits: AtomicU32, next: &'static Node }
struct Leaf { tag: &'static str, n: u32 }
static LEAF: Leaf = Leaf { tag: "leaf", n: 3 };
static SH1: Node = Node { id: 1, hits: AtomicU32::new(0), next: &SH2 };
static SH2: Node = Node { id: 2, hits: AtomicU32::new(0), next: &SH1 };

struct Rich { id: u32, leaf: &'static Leaf, call: Option<fn()>, next: &'static Rich }
static AO1: Rich = Rich { id: 11, leaf: &LEAF, call: None, next: &AO2 };
static AO2: Rich = Rich { id: 12, leaf: &LEAF, call: None, next: &AO3 };
static AO3: Rich = Rich { id: 13, leaf: &LEAF, call: None, next: &AO1 };
struct Hop { id: u32, call: fn(), next: &'static Hop }
static AH1: Hop = Hop { id: 14, call: chunk_c, next: &AH2 };
static AH2: Hop = Hop { id: 15, call: chunk_c, next: &AH1 };
static A_TO_SH: &Node = &SH2;
struct Me { id: u32, me: &'static Me }
static A_SELF: Me = Me { id: 16, me: &A_SELF };

struct Bo { id: u32, call: fn(), leaf: &'static Leaf }
static BO: Bo = Bo { id: 21, call: chunk_a, leaf: &LEAF };
struct BTab { nodes: [&'static Node; 2], bo: &'static Bo, me: &'static BTab }
static B_TAB: BTab = BTab { nodes: [&SH2, &SH1], bo: &BO, me: &B_TAB };

static CDE1: Hop = Hop { id: 31, call: chunk_e, next: &CDE2 };
static CDE2: Hop = Hop { id: 32, call: chunk_e, next: &CDE1 };
static C_OWN: [&'static Hop; 2] = [&CDE2, &CDE1];

static E1: Me = Me { id: 51, me: &E2 };
static E2: Me = Me { id: 52, me: &E1 };

static SHT: [fn(); 3] = [chunk_b, chunk_c, chunk_e];
static DEPTH: AtomicU32 = AtomicU32::new(0);
static CALLS: AtomicU32 = AtomicU32::new(0);

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("a");
    CALLS.fetch_add(1, Ordering::Relaxed);
    SH1.next.hits.fetch_add(5, Ordering::Relaxed);
    print_u32(SH1.id * 10 + SH1.next.next.id);
    print_u32(AO1.next.next.id + AO3.next.id * 100 + AO2.leaf.n * 10000);
    print_bool(core::ptr::eq(AO3.next, &AO1));
    print_bool(core::ptr::eq(AO1.leaf, AO3.leaf));
    print_str(AO2.next.leaf.tag);
    print_u32(A_TO_SH.next.id * 10 + A_TO_SH.id);
    print_bool(core::ptr::eq(A_TO_SH, &SH2));
    print_u32(A_SELF.me.me.id);
    print_u32(AH1.next.id + AH2.next.id);
    if DEPTH.fetch_add(1, Ordering::Relaxed) < 1 {
        (AH1.next.call)();
        (SHT[0])();
    }
}
#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("b");
    CALLS.fetch_add(1, Ordering::Relaxed);
    print_u32(SH2.hits.load(Ordering::Relaxed));
    SH2.next.next.hits.fetch_add(1, Ordering::Relaxed);
    print_u32(B_TAB.nodes[0].id * 10 + B_TAB.me.nodes[1].next.id + B_TAB.bo.id * 100 + B_TAB.bo.leaf.n * 10000);
    print_bool(core::ptr::eq(B_TAB.me, &B_TAB));
    print_bool(core::ptr::eq(B_TAB.nodes[1], &SH1));
    if DEPTH.fetch_add(1, Ordering::Relaxed) < 3 {
        (BO.call)();
    }
}
#[unsafe(no_mangle)]
pub fn chunk_c() {
    print_str("c");
    CALLS.fetch_add(1, Ordering::Relaxed);
    print_u32(C_OWN[0].id + C_OWN[1].next.id * 100 + CDE1.next.next.id * 10000);
    print_bool(core::ptr::eq(C_OWN[0], &CDE2));
    print_bool(core::ptr::eq(C_OWN[1].next, C_OWN[0]));
}
#[unsafe(no_mangle)]
pub fn chunk_d() {
    print_str("d");
    CALLS.fetch_add(1, Ordering::Relaxed);
    print_u32(CDE2.next.id);
    chunk_e();
    (CDE1.call)();
    print_u32(E1.me.me.id);
}
#[unsafe(no_mangle)]
pub fn chunk_e() {
    print_str("e");
    CALLS.fetch_add(1, Ordering::Relaxed);
    print_u32(E2.me.id * 100 + E1.me.id + CDE2.id * 10000);
    print_bool(core::ptr::eq(E2.me.me, &E2));
    print_u32(CALLS.load(Ordering::Relaxed));
    print_u32(LEAF.n);
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
    chunk_c();
    chunk_d();
    chunk_e();
}
