//! (f) interior mutability: Cell<Struct> set/replace/swap/take, RefCell whole stores.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::hint::black_box;
use prelude::*;

#[derive(Clone, Copy, Default)]
struct H {
    head: u32,
    tail: u32,
}
#[derive(Default)]
struct Big {
    n: u32,
    h: H,
    v: Vec<u32>,
}

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

fn h(x: u32) -> H {
    H { head: x, tail: x + 1 }
}

#[inline(never)]
fn e_cell() {
    let c = Cell::new(h(1));
    let at = c.as_ptr() as usize;
    c.set(h(2));
    print_bool(c.as_ptr() as usize == at);
    print_bool(a(&c) == at);
    let old = c.replace(h(3));
    print_u32(old.head * 10 + c.get().head);
    let t = c.take();
    print_u32(t.head * 10 + c.get().head);
    let c2 = Cell::new(h(7));
    let at2 = c2.as_ptr() as usize;
    c.swap(&c2);
    print_u32(c.get().head * 10 + c2.get().head);
    print_bool(c.as_ptr() as usize == at);
    print_bool(c2.as_ptr() as usize == at2);
    let got = c.get();
    c.set(h(9));
    print_u32(got.head * 10 + c.get().head);
    // through a shared reference into a struct
    struct WithCell {
        n: u32,
        c: Cell<H>,
    }
    let w = black_box(WithCell { n: 5, c: Cell::new(h(4)) });
    let shared = &w;
    let field = a(&shared.c);
    shared.c.set(h(6));
    print_bool(a(&w.c) == field);
    print_u32(w.c.get().head + w.n);
    // Cell<Option<H>> and Cell<[u32; 2]>
    let co: Cell<Option<H>> = Cell::new(Some(h(1)));
    let x = co.take();
    co.set(Some(h(2)));
    print_u32(x.map(|h| h.head).unwrap_or(0) * 10 + co.get().map(|h| h.head).unwrap_or(0));
    let ca = Cell::new([1u32, 2]);
    let ata = ca.as_ptr() as usize;
    ca.set([3, 4]);
    print_bool(ca.as_ptr() as usize == ata);
    print_u32(ca.get()[0] + ca.get()[1]);
}

#[inline(never)]
fn e_refcell() {
    let rc = RefCell::new(black_box(Big { n: 1, h: h(1), v: vec![1] }));
    let atr = rc.as_ptr() as usize;
    let ath = a(&rc.borrow().h);
    *rc.borrow_mut() = Big { n: 2, h: h(2), v: vec![2, 2] };
    print_bool(rc.as_ptr() as usize == atr);
    print_bool(a(&*rc.borrow()) == atr);
    print_bool(a(&rc.borrow().h) == ath);
    print_usize(rc.borrow().v.len());
    rc.borrow_mut().h = h(3);
    rc.borrow_mut().v.push(1);
    print_u32(rc.borrow().h.head + rc.borrow().v[2]);
    let old = rc.replace(Big { n: 4, h: h(4), v: vec![] });
    print_u32(old.n * 10 + rc.borrow().n);
    print_usize(old.v.len() * 10 + rc.borrow().v.len());
    let t = rc.take();
    print_u32(t.n * 10 + rc.borrow().n);
    {
        let mut b = rc.borrow_mut();
        let inner: &mut Big = &mut *b;
        *inner = Big { n: 5, h: h(5), v: vec![5, 5, 5] };
    }
    print_bool(a(&*rc.borrow()) == atr);
    print_bool(a(&rc.borrow().h) == ath);
    print_u32(rc.borrow().n + rc.borrow().v.len() as u32);
    let rv: RefCell<Vec<H>> = RefCell::new(vec![h(1), h(2)]);
    rv.borrow_mut()[1] = h(3);
    let e1 = a(&rv.borrow()[1]);
    rv.borrow_mut()[1] = h(4);
    print_bool(a(&rv.borrow()[1]) == e1);
    print_u32(rv.borrow()[1].head);
    let swapped = rv.replace_with(|v| {
        let mut n = v.clone();
        n.push(h(9));
        n
    });
    print_usize(swapped.len() * 10 + rv.borrow().len());
    // RefCell inside a struct behind &, field of the RefCell'd struct reassigned
    struct Node {
        k: u32,
        payload: RefCell<H>,
    }
    let node = black_box(Node { k: 1, payload: RefCell::new(h(1)) });
    let r = &node;
    let ap = a(&*r.payload.borrow());
    *r.payload.borrow_mut() = h(8);
    print_bool(a(&*node.payload.borrow()) == ap);
    print_u32(node.payload.borrow().head + node.k);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_cell();
    e_refcell();
}
