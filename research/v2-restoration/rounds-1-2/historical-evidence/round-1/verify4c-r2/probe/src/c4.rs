//! Verifier round-2 probe c4: initialization order across chunks for the reworked `order`.
//! Statics owned by one chunk that read pooled bindings owned by the shared chunk, by their own chunk,
//! statics that read other statics (chains and a cycle), and function pointers in statics whose
//! functions read pooled bindings.
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Row { name: &'static str, weight: u32 }

macro_rules! table { ($name:ident, $tag:literal, $base:expr) => {
    const $name: &[Row] = &[
        Row { name: concat!($tag, "-row-0-with-a-longish-name-to-reach-the-threshold"), weight: $base },
        Row { name: concat!($tag, "-row-1-with-a-longish-name-to-reach-the-threshold"), weight: $base + 1 },
        Row { name: concat!($tag, "-row-2-with-a-longish-name-to-reach-the-threshold"), weight: $base + 2 },
        Row { name: concat!($tag, "-row-3-with-a-longish-name-to-reach-the-threshold"), weight: $base + 3 },
        Row { name: concat!($tag, "-row-4-with-a-longish-name-to-reach-the-threshold"), weight: $base + 4 },
    ];
}; }
table!(T_AB, "c4-ab", 10);   // read by functions of both chunks
table!(T_A, "c4-a", 100);    // read by chunk a's functions only (twice)
table!(T_B, "c4-b", 200);    // read by chunk b's functions only (twice)
table!(T_C, "c4-c", 300);    // read by chunk c's functions only, and by a static of chunk a

#[inline(never)] fn sum(rows: &[Row]) -> u32 { rows.iter().map(|r| r.weight).sum() }
#[inline(never)] fn read_ab() -> u32 { sum(T_AB) }
#[inline(never)] fn read_a() -> u32 { sum(T_A) + T_A[2].weight }
#[inline(never)] fn read_b() -> u32 { sum(T_B) + T_B[3].weight }
#[inline(never)] fn read_c() -> u32 { sum(T_C) + T_C[4].weight }

// chunk a only: a static pointing at the shared pooled table, one at a's own, one at c's.
static A_IDX_SHARED: &[Row] = T_AB;
static A_IDX_OWN: &[Row] = T_A;
static A_IDX_OTHER: &[Row] = T_C;
// chunk a only: a static reading another static of chunk a, which reads the pooled table.
static A_CHAIN: &&[Row] = &A_IDX_OWN;
// chunk b only: function pointers whose functions read pooled bindings of other owners.
static B_FNS: [fn() -> u32; 3] = [read_ab, read_a, read_c];
// both chunks: a static chain into a pooled table only b reads otherwise.
static SHARED_IDX: (&[Row], &[Row]) = (T_B, T_AB);
// a cycle between a static only chunk a names and one only chunk b names.
struct Node { next: &'static Node, rows: &'static [Row], id: u32 }
static NODE_A: Node = Node { next: &NODE_B, rows: T_A, id: 1 };
static NODE_B: Node = Node { next: &NODE_A, rows: T_B, id: 2 };
// chunk c only: a static whose value is a slice of statics.
static C_NODES: &[&Node] = &[&NODE_A, &NODE_B];

#[unsafe(no_mangle)]
pub fn chunk_a() {
    print_str("chunk a");
    print_u32(read_ab()); print_u32(read_a()); print_u32(read_a());
    print_u32(sum(A_IDX_SHARED)); print_u32(sum(A_IDX_OWN)); print_u32(sum(A_IDX_OTHER)); print_u32(sum(*A_CHAIN));
    print_u32(sum(SHARED_IDX.0) + sum(SHARED_IDX.1));
    print_u32(NODE_A.id + NODE_A.next.id * 10 + NODE_A.next.next.id * 100 + sum(NODE_A.next.rows));
    print_bool(core::ptr::eq(NODE_A.next.next, &NODE_A));
    print_bool(core::ptr::eq(A_IDX_OWN.as_ptr(), (*A_CHAIN).as_ptr()));
}
#[unsafe(no_mangle)]
pub fn chunk_b() {
    print_str("chunk b");
    print_u32(read_ab()); print_u32(read_b()); print_u32(read_b());
    for f in B_FNS.iter() { print_u32(f()); }
    print_u32(sum(SHARED_IDX.0) * 2 + sum(SHARED_IDX.1));
    print_u32(NODE_B.id + NODE_B.next.id * 10 + sum(NODE_B.next.rows) + sum(NODE_B.rows));
}
#[unsafe(no_mangle)]
pub fn chunk_c() {
    print_str("chunk c");
    print_u32(read_c()); print_u32(read_c());
    let mut t = 0; for n in C_NODES { t += n.id + sum(n.rows) + n.next.id; } print_u32(t);
}
#[unsafe(no_mangle)]
pub fn rust_entry() { chunk_a(); chunk_b(); chunk_c(); chunk_a(); }
