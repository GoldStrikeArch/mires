//! Statics that refer to each other initialize, and keep their identity.
//!
//! A `static` is a module level `let` whose initializer names the bindings it points at, so a
//! binding has to be declared after the ones it names. Rust allows statics that name each other —
//! two nodes, a ring of three, a static that names itself — and no order of plain `let`s
//! initializes those: the program used to throw a `ReferenceError` (temporal dead zone) before
//! `rust_entry` ran. The link step now declares every member of such a cycle first and fills each
//! one in afterwards (`jsc-link/src/link/statics.rs`).
//!
//! What is pinned: walks around every cycle; `ptr::eq` between a member reached through the cycle
//! and the member named directly; statics outside a cycle that point into it (`INTO`, `CHAIN`);
//! members that hold a leaf static and long strings (hoisted into shared constants); a cycle
//! through a static slice of references (`LIST`), through an array of references and a static that
//! is itself only a reference (`PEERS`, `FIRST`, a boxed binding); and a `const` table naming the
//! members of a cycle. A constant that a static it names would use is a query cycle Rust refuses,
//! so `ALL` is read by code, where it is decoded inline.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O3`
//! and `-C opt-level=0`, which agree.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

struct Node {
    id: u32,
    next: &'static Node,
}
static A: Node = Node { id: 1, next: &B };
static B: Node = Node { id: 2, next: &A };

static X: Node = Node { id: 10, next: &Y };
static Y: Node = Node { id: 20, next: &Z };
static Z: Node = Node { id: 30, next: &X };

struct Me {
    id: u32,
    me: &'static Me,
}
static S: Me = Me { id: 99, me: &S };

/// Outside every cycle: a reference into one, and an array that reaches two.
static INTO: &Node = &Y;
static CHAIN: [&Node; 3] = [&A, &X, INTO];

struct Leaf {
    tag: &'static str,
}
static LEAF: Leaf = Leaf { tag: "a-leaf-with-a-long-name-so-the-string-is-hoisted-out" };
struct Rich {
    id: u32,
    leaf: &'static Leaf,
    label: &'static str,
    peer: &'static Rich,
}
static R1: Rich = Rich {
    id: 5,
    leaf: &LEAF,
    label: "rich-one-with-a-long-label-so-it-is-hoisted",
    peer: &R2,
};
static R2: Rich = Rich { id: 6, leaf: &LEAF, label: "short", peer: &R1 };

/// A static slice of references naming statics that name it back.
static LIST: &[&Item] = &[&I1, &I2];
struct Item {
    id: u32,
    list: &'static &'static [&'static Item],
}
static I1: Item = Item { id: 7, list: &LIST };
static I2: Item = Item { id: 8, list: &LIST };

/// An array of references, and a static that is only a reference.
struct Ring {
    id: u32,
    peers: &'static [&'static Ring; 2],
    first: &'static &'static Ring,
}
static PEERS: [&Ring; 2] = [&R0, &RR];
static FIRST: &Ring = &R0;
static R0: Ring = Ring { id: 40, peers: &PEERS, first: &FIRST };
static RR: Ring = Ring { id: 41, peers: &PEERS, first: &FIRST };

struct Spoke {
    id: u32,
    hub: &'static Hub,
}
struct Hub {
    id: u32,
    all: &'static [&'static Spoke],
}
const ALL: &[&Spoke] = &[&SP1, &SP2, &SP3];
static HUB: Hub = Hub { id: 9, all: &[&SP1, &SP2, &SP3] };
static SP1: Spoke = Spoke { id: 1, hub: &HUB };
static SP2: Spoke = Spoke { id: 2, hub: &HUB };
static SP3: Spoke = Spoke { id: 3, hub: &HUB };

fn walk(mut n: &'static Node, steps: u32) -> u32 {
    let mut t = 0;
    for _ in 0..steps {
        t = t * 100 + n.id;
        n = n.next;
    }
    t
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(walk(&A, 4));
    print_bool(core::ptr::eq(A.next.next, &A));
    print_bool(core::ptr::eq(B.next, &A));
    print_u32(walk(&X, 4));
    print_bool(core::ptr::eq(Z.next.next.next, &Z));
    print_u32(S.me.me.id);
    print_bool(core::ptr::eq(S.me, &S));

    print_u32(walk(INTO, 3));
    print_bool(core::ptr::eq(INTO, &Y));
    let mut t = 0;
    for n in CHAIN.iter() {
        t = t * 10 + n.next.id;
    }
    print_u32(t);
    print_bool(core::ptr::eq(CHAIN[2].next, &Z));

    print_str(R1.peer.peer.leaf.tag);
    print_str(R1.peer.label);
    print_str(R2.peer.label);
    print_u32(R2.peer.id + R1.peer.id);
    print_bool(core::ptr::eq(R1.leaf, R2.leaf));
    print_bool(core::ptr::eq(R2.peer, &R1));

    print_u32(I1.list[1].id * 100 + I2.list[0].id);
    print_bool(core::ptr::eq(I1.list[0], &I1));
    print_bool(core::ptr::eq(*I2.list, *I1.list));

    print_u32(R0.peers[1].id + RR.first.id * 100 + FIRST.peers[1].first.id);
    print_bool(core::ptr::eq(RR.peers[0], &R0));
    print_bool(core::ptr::eq(*R0.first, &R0));
    print_bool(core::ptr::eq(R0.first, &FIRST));
    print_bool(core::ptr::eq(R0.peers, &PEERS));

    let mut t = 0;
    for n in HUB.all {
        t = t * 10 + n.id;
    }
    for n in ALL {
        t = t * 10 + n.hub.all[n.id as usize - 1].id;
    }
    print_u32(t + SP1.hub.id);
    print_bool(core::ptr::eq(HUB.all[1], &SP2));
    print_bool(core::ptr::eq(SP2.hub.all[0].hub, &HUB));
    print_bool(core::ptr::eq(ALL[0], &SP1));
    print_u32(ALL[2].hub.all[1].id);
}
