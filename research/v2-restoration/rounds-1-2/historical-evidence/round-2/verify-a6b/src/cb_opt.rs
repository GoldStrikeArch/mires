#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cell::Cell;

// Cycles through niche-encoded and tagged enum payloads, nested structs, tables and slices.
struct Row { id: u32, parent: Option<&'static Row>, kids: &'static [Option<&'static Row>], table: Option<&'static [Row; 3]> }
static ROWS: [Row; 3] = [
    Row { id: 1, parent: None, kids: &[Some(&ROOT), None], table: Some(&ROWS) },
    Row { id: 2, parent: Some(&ROOT), kids: &[], table: None },
    Row { id: 3, parent: Some(&ROOT), kids: &[None, Some(&ROOT), Some(&LONE)], table: Some(&ROWS) },
];
static ROOT: Row = Row { id: 10, parent: None, kids: &[Some(&LONE), Some(&ROOT)], table: Some(&ROWS) };
static LONE: Row = Row { id: 20, parent: Some(&LONE), kids: &[], table: None };

struct Deep { id: u32, a: Mid }
struct Mid { tag: u8, b: Inner, pair: Option<(&'static Deep, &'static Deep)> }
struct Inner { link: Option<Option<&'static Deep>>, res: Result<&'static Deep, &'static Leaf>, rr: Option<&'static &'static Deep> }
struct Leaf { n: u32 }
static LEAF: Leaf = Leaf { n: 7 };
static D1: Deep = Deep { id: 1, a: Mid { tag: 1, b: Inner { link: Some(Some(&D2)), res: Ok(&D2), rr: Some(&D2R) }, pair: Some((&D2, &D1)) } };
static D2: Deep = Deep { id: 2, a: Mid { tag: 2, b: Inner { link: Some(None), res: Err(&LEAF), rr: None }, pair: Some((&D1, &D1)) } };
static D3: Deep = Deep { id: 3, a: Mid { tag: 3, b: Inner { link: None, res: Ok(&D3), rr: Some(&D3R) }, pair: None } };
static D2R: &Deep = &D2;
static D3R: &Deep = &D3;

enum Tree { Leaf(u32), Node(&'static Tree, &'static Tree), Back(Option<&'static Tree>), Many(&'static [&'static Tree]) }
static T1: Tree = Tree::Node(&T2, &T3);
static T2: Tree = Tree::Back(Some(&T1));
static T3: Tree = Tree::Many(&[&T4, &T2, &T5]);
static T4: Tree = Tree::Leaf(4);
static T5: Tree = Tree::Back(None);
static TS: Tree = Tree::Many(&[&TS, &T4]);

struct Str { s: Option<&'static str>, sl: Option<&'static [u8]>, next: Option<&'static Str>, f: Option<fn(&'static Str) -> u32> }
fn len_of(x: &'static Str) -> u32 { x.s.map(|s| s.len() as u32).unwrap_or(99) }
static S1: Str = Str { s: Some("one"), sl: Some(b"ab"), next: Some(&S2), f: Some(len_of) };
static S2: Str = Str { s: None, sl: None, next: Some(&S1), f: None };

struct CellN { id: u32, cur: Cell<Option<&'static CellN>>, next: Option<&'static CellN> }
unsafe impl Sync for CellN {}
static C1: CellN = CellN { id: 1, cur: Cell::new(Some(&C2)), next: Some(&C2) };
static C2: CellN = CellN { id: 2, cur: Cell::new(None), next: Some(&C1) };

fn tree(t: &'static Tree, depth: u32) -> u32 {
    if depth == 0 { return 1; }
    match t {
        Tree::Leaf(n) => *n,
        Tree::Node(a, b) => 2 * tree(a, depth - 1) + 3 * tree(b, depth - 1),
        Tree::Back(Some(b)) => 5 + tree(b, depth - 1),
        Tree::Back(None) => 7,
        Tree::Many(all) => { let mut s = 0; for x in all.iter() { s = s * 2 + tree(x, depth - 1); } s }
    }
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut t = 0;
    for r in ROWS.iter() {
        t = t * 10 + r.id + r.parent.map(|p| p.id).unwrap_or(5);
        for k in r.kids.iter() { t = t * 3 + k.map(|k| k.id).unwrap_or(1); }
        t += r.table.map(|tb| tb[2].id).unwrap_or(0);
    }
    print_u32(t);
    print_bool(core::ptr::eq(ROWS[2].kids[1].unwrap(), &ROOT));
    print_bool(core::ptr::eq(ROOT.table.unwrap(), &ROWS));
    print_bool(core::ptr::eq(ROOT.kids[1].unwrap().kids[0].unwrap(), &LONE));
    print_bool(core::ptr::eq(LONE.parent.unwrap().parent.unwrap(), &LONE));
    print_bool(ROWS[0].parent.is_none() && ROWS[1].table.is_none() && ROWS[1].parent.is_some());
    print_usize(ROOT.kids.len() + ROWS[2].kids.len() * 10 + ROWS[1].kids.len() * 100);

    print_u32(D1.a.b.link.unwrap().unwrap().id + D1.a.pair.unwrap().1.id * 10 + D2.a.pair.unwrap().0.a.tag as u32 * 100);
    print_bool(D2.a.b.link.unwrap().is_none());
    print_bool(D3.a.b.link.is_none() && D3.a.pair.is_none() && D2.a.b.rr.is_none());
    print_u32(match D1.a.b.res { Ok(d) => d.id, Err(l) => l.n * 100 } + match D2.a.b.res { Ok(d) => d.id, Err(l) => l.n * 100 });
    print_bool(core::ptr::eq(*D1.a.b.rr.unwrap(), &D2));
    print_bool(core::ptr::eq(D1.a.b.rr.unwrap(), &D2R));
    print_bool(core::ptr::eq(D3.a.b.res.ok().unwrap(), &D3));
    print_bool(core::ptr::eq(*D3.a.b.rr.unwrap(), &D3));
    print_bool(core::ptr::eq(D2.a.pair.unwrap().0.a.pair.unwrap().0, &D2));

    print_u32(tree(&T1, 6));
    print_u32(tree(&TS, 4));
    if let Tree::Many(all) = &TS { print_bool(core::ptr::eq(all[0], &TS)); }
    if let Tree::Back(Some(b)) = &T2 { print_bool(core::ptr::eq(*b, &T1)); }

    print_u32((S1.f.unwrap())(&S1) + (S1.f.unwrap())(S1.next.unwrap()) * 100 + S1.sl.unwrap().len() as u32 * 10000);
    print_bool(S2.f.is_none() && S2.s.is_none() && S2.sl.is_none());
    print_bool(core::ptr::eq(S2.next.unwrap().next.unwrap(), &S2));
    print_str(S2.next.unwrap().s.unwrap());

    print_u32(C1.cur.get().map(|c| c.id).unwrap_or(9) * 10 + C2.cur.get().map(|c| c.id).unwrap_or(9));
    C2.cur.set(C1.next);
    C1.next.unwrap().next.unwrap().cur.set(None);
    print_u32(C1.cur.get().map(|c| c.id).unwrap_or(9) * 10 + C2.cur.get().map(|c| c.id).unwrap_or(9));
    print_bool(core::ptr::eq(C2.cur.get().unwrap(), &C2));
}
