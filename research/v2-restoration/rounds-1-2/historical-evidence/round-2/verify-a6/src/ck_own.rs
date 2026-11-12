#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};

// Three chunks that never call each other, so each really owns statics.
//  shared: SH ring of three (a, b, c reach it), LEAF, SHARED_TAB (names SH members), CNT
//  a: A1 <-> A2 cycle pointing into the shared ring and at LEAF; A_ARR self-referencing array;
//     A_PTR boxed reference to a shared static; A_CHAIN (acyclic, a-owned -> a-owned -> shared)
//  b: B_SELF; B_LIST slice of refs naming a b-owned cycle and shared statics; static mut B_MUT
//  c: C1 -> C2 -> C3 -> C1 ring whose members hold a shared slice const and a fn pointer to a
//     c-owned function; C_TOP outside the ring naming it
//  ab-shared: AB cycle reached by a and b only
struct Node { id: u32, hits: AtomicU32, next: &'static Node }
struct Leaf { tag: &'static str, n: u32 }
static LEAF: Leaf = Leaf { tag: "leaf-shared", n: 3 };
static SH1: Node = Node { id: 1, hits: AtomicU32::new(0), next: &SH2 };
static SH2: Node = Node { id: 2, hits: AtomicU32::new(0), next: &SH3 };
static SH3: Node = Node { id: 3, hits: AtomicU32::new(0), next: &SH1 };
static SHARED_TAB: [&Node; 3] = [&SH3, &SH1, &SH2];
static CNT: AtomicU32 = AtomicU32::new(0);

struct Own { id: u32, into: &'static Node, leaf: &'static Leaf, peer: &'static Own, tab: &'static [&'static Node; 3] }
static A1: Own = Own { id: 11, into: &SH2, leaf: &LEAF, peer: &A2, tab: &SHARED_TAB };
static A2: Own = Own { id: 12, into: &SH3, leaf: &LEAF, peer: &A1, tab: &SHARED_TAB };
struct El { id: u32, all: &'static [El; 2], own: &'static Own }
static A_ARR: [El; 2] = [El { id: 13, all: &A_ARR, own: &A2 }, El { id: 14, all: &A_ARR, own: &A1 }];
static A_PTR: &Node = &SH1;
struct Chain { id: u32, next: Option<&'static Chain>, node: &'static Node }
static A_CH3: Chain = Chain { id: 17, next: None, node: &SH3 };
static A_CH2: &Chain = &A_CH3;
static A_CH1: [&&Chain; 2] = [&A_CH2, &A_CH2];

struct Me { id: u32, me: &'static Me, leaf: &'static Leaf }
static B_SELF: Me = Me { id: 21, me: &B_SELF, leaf: &LEAF };
struct BN { id: u32, list: &'static &'static [&'static BN], sh: &'static Node }
static B_LIST: &[&BN] = &[&BN1, &BN2];
static BN1: BN = BN { id: 22, list: &B_LIST, sh: &SH1 };
static BN2: BN = BN { id: 23, list: &B_LIST, sh: &SH2 };
struct MN { count: u32, next: *mut MN }
static mut B_MUT1: MN = MN { count: 1, next: unsafe { core::ptr::addr_of_mut!(B_MUT2) } };
static mut B_MUT2: MN = MN { count: 2, next: unsafe { core::ptr::addr_of_mut!(B_MUT1) } };

struct CR { id: u32, f: fn(u32) -> u32, sl: &'static [&'static Node], next: &'static CR }
fn c_helper(x: u32) -> u32 { x * 3 + SH1.id }
const SLC: &[&Node] = &[&SH2, &SH3];
static C1: CR = CR { id: 31, f: c_helper, sl: SLC, next: &C2 };
static C2: CR = CR { id: 32, f: c_helper, sl: SLC, next: &C3 };
static C3: CR = CR { id: 33, f: c_helper, sl: &[], next: &C1 };
static C_TOP: [&CR; 2] = [&C3, &C1];

static AB1: Me = Me { id: 41, me: &AB2, leaf: &LEAF };
static AB2: Me = Me { id: 42, me: &AB1, leaf: &LEAF };

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("a");
    CNT.fetch_add(1, Ordering::Relaxed);
    A1.into.hits.fetch_add(7, Ordering::Relaxed);
    print_u32(A1.peer.id + A2.peer.peer.id * 100 + A1.into.next.id * 10000);
    print_bool(core::ptr::eq(A1.peer.peer, &A1));
    print_bool(core::ptr::eq(A1.tab, A2.tab));
    print_bool(core::ptr::eq(A2.tab[0], A2.into));
    print_str(A2.peer.leaf.tag);
    print_u32(A_ARR[0].all[1].id + A_ARR[1].own.id * 100 + A_ARR[1].all[0].own.peer.id * 10000);
    print_bool(core::ptr::eq(A_ARR[1].all, &A_ARR));
    print_u32(A_PTR.next.next.id + A_PTR.id * 10);
    print_u32(A_CH1[1].id + A_CH1[0].node.id * 100 + A_CH1[0].next.map(|c| c.id).unwrap_or(5) * 10000);
    print_u32(AB1.me.id + AB2.me.me.id * 100);
}
#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("b");
    CNT.fetch_add(10, Ordering::Relaxed);
    print_u32(SH2.hits.load(Ordering::Relaxed));
    print_u32(B_SELF.me.me.id + B_SELF.me.leaf.n * 100);
    print_bool(core::ptr::eq(B_SELF.me, &B_SELF));
    print_u32(B_LIST[0].id + B_LIST[1].list[0].sh.id * 100 + BN2.list[1].sh.next.id * 10000);
    print_bool(core::ptr::eq(BN1.list[1], &BN2));
    print_bool(core::ptr::eq(*BN1.list, *BN2.list));
    unsafe {
        (*B_MUT1.next).count += 40;
        (*(*B_MUT2.next).next).count += 500;
        print_u32(B_MUT1.count + B_MUT2.count * 10);
        print_bool((*B_MUT1.next).next == core::ptr::addr_of_mut!(B_MUT1));
    }
    print_bool(core::ptr::eq(AB2.me.me, &AB2));
    print_u32(AB1.leaf.n);
}
#[unsafe(no_mangle)]
pub fn chunk_c() {
    print_str("c");
    CNT.fetch_add(100, Ordering::Relaxed);
    print_u32(C1.next.next.id + (C2.f)(C3.next.id) * 100);
    print_usize(C1.sl.len() + C3.sl.len() * 10 + C_TOP.len() * 100);
    print_u32(C2.sl[1].id + C_TOP[0].next.sl[0].id * 10 + C_TOP[1].next.next.next.id * 100);
    print_bool(core::ptr::eq(C_TOP[0].next, C_TOP[1]));
    print_bool(core::ptr::eq(C1.sl[0], &SH2));
    print_u32(CNT.load(Ordering::Relaxed));
    print_u32(SH1.next.hits.load(Ordering::Relaxed));
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    chunk_a();
    chunk_b();
    chunk_c();
}
