#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

enum L { End, More(u32, &'static L) }
static L1: L = L::More(1, &L2);
static L2: L = L::More(2, &L3);
static L3: L = L::More(3, &L1);
static LE: L = L::End;
enum M { A(&'static M), B(&'static M), C }
static M1: M = M::A(&M2);
static M2: M = M::B(&M1);
static MS: M = M::A(&MS);
struct N { id: u32, o: &'static Option<&'static N> }
static O: Option<&'static N> = Some(&N1);
static N1: N = N { id: 5, o: &O };
static ON: Option<&'static N> = None;
struct Q { id: u32, k: Kind }
enum Kind { Leaf, One(&'static Q), Two(&'static Q, &'static Q), Named { a: u8, q: &'static Q } }
static Q1: Q = Q { id: 1, k: Kind::One(&Q2) };
static Q2: Q = Q { id: 2, k: Kind::Two(&Q1, &Q3) };
static Q3: Q = Q { id: 3, k: Kind::Named { a: 9, q: &Q1 } };
static Q4: Q = Q { id: 4, k: Kind::Leaf };
fn walk(mut l: &'static L, n: u32) -> u32 {
    let mut t = 0;
    for _ in 0..n { match l { L::End => { t += 1000; break } L::More(v, next) => { t = t * 10 + v; l = next; } } }
    t
}
fn m(x: &'static M, depth: u32) -> u32 {
    if depth == 0 { return 0; }
    match x { M::A(n) => 1 + 10 * m(n, depth - 1), M::B(n) => 2 + 10 * m(n, depth - 1), M::C => 3 }
}
fn q(x: &'static Q, depth: u32) -> u32 {
    if depth == 0 { return x.id; }
    match &x.k {
        Kind::Leaf => x.id * 7,
        Kind::One(a) => x.id + 10 * q(a, depth - 1),
        Kind::Two(a, b) => x.id + 10 * q(a, depth - 1) + 100 * q(b, depth - 1),
        Kind::Named { a, q: n } => x.id + *a as u32 * 1000 + q(n, depth - 1),
    }
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(walk(&L1, 7));
    print_u32(walk(&LE, 7));
    if let L::More(_, n) = &L3 { print_bool(core::ptr::eq(*n, &L1)); }
    print_u32(m(&M1, 5));
    print_u32(m(&MS, 4));
    if let M::A(n) = &MS { print_bool(core::ptr::eq(*n, &MS)); }
    if let M::B(n) = &M2 { print_bool(core::ptr::eq(*n, &M1)); }
    print_u32(O.unwrap().id + N1.o.unwrap().o.unwrap().id);
    print_bool(core::ptr::eq(O.unwrap(), &N1));
    print_bool(core::ptr::eq(N1.o, &O));
    print_bool(ON.is_none());
    print_bool(N1.o.is_some());
    print_u32(q(&Q1, 4));
    print_u32(q(&Q3, 3));
    print_u32(q(&Q4, 3));
    if let Kind::Two(a, b) = &Q2.k { print_bool(core::ptr::eq(*a, &Q1)); print_bool(core::ptr::eq(*b, &Q3)); }
}
