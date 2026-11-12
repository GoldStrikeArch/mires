//! (j) whole-value stores into places that hold no object yet, or hold `None`: MaybeUninit
//! elements and fields, Option fields going None -> Some -> None -> Some, late-initialised
//! locals, a struct built through a raw pointer field by field.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem::MaybeUninit;
use prelude::*;

#[derive(Clone, Copy)]
struct S {
    a: u32,
    b: u32,
}
struct N {
    a: u32,
    b: u32,
}
struct Pair {
    s: S,
    n: N,
}
struct Slot {
    full: bool,
    v: MaybeUninit<S>,
    w: MaybeUninit<Pair>,
}
struct Opts {
    k: u32,
    o: Option<S>,
    on: Option<N>,
    ob: Option<Box<N>>,
    op: Option<Pair>,
    r: Result<S, N>,
}
#[inline(never)]
fn mk(a: u32) -> S {
    black_box(S { a, b: a + 1 })
}
#[inline(never)]
fn mkn(a: u32) -> N {
    black_box(N { a, b: a + 1 })
}
#[inline(never)]
fn uninit_s() -> MaybeUninit<S> {
    black_box(MaybeUninit::uninit())
}
fn ad<T>(r: &T) -> usize {
    r as *const T as usize
}

#[inline(never)]
fn t_maybe_uninit() {
    print_str("maybe_uninit");
    let mut arr: [MaybeUninit<S>; 3] = [MaybeUninit::uninit(); 3];
    arr[0] = MaybeUninit::new(mk(1));
    arr[1].write(mk(2));
    arr[2] = MaybeUninit::new(S { a: 3, b: 4 });
    arr[0] = MaybeUninit::new(mk(5));
    let v = unsafe { [arr[0].assume_init(), arr[1].assume_init(), arr[2].assume_init()] };
    print_u32(v[0].a * 100 + v[1].a * 10 + v[2].a);
    let mut an: [MaybeUninit<N>; 2] = [const { MaybeUninit::uninit() }; 2];
    an[1] = MaybeUninit::new(mkn(7));
    an[0] = MaybeUninit::new(N { a: 8, b: 9 });
    let [x, y] = an;
    print_u32(unsafe { x.assume_init().a * 10 + y.assume_init().b });
    let mut slot = black_box(Slot { full: false, v: MaybeUninit::uninit(), w: MaybeUninit::uninit() });
    slot.v = MaybeUninit::new(mk(4));
    slot.w = MaybeUninit::new(Pair { s: mk(1), n: mkn(2) });
    slot.full = true;
    slot.v = MaybeUninit::new(mk(6));
    print_u32(unsafe { slot.v.assume_init().a * 100 + slot.w.assume_init_ref().n.a * 10 + slot.w.assume_init_ref().s.b });
    let mut late = MaybeUninit::<S>::uninit();
    late = MaybeUninit::new(mk(9));
    late.write(mk(8));
    print_u32(unsafe { late.assume_init().a });
    let mut v: Vec<MaybeUninit<S>> = Vec::new();
    v.push(uninit_s());
    v.push(MaybeUninit::new(mk(1)));
    v[0] = MaybeUninit::new(mk(2));
    v.swap(0, 1);
    print_u32(unsafe { v[0].assume_init().a * 10 + v[1].assume_init().a });
}

#[inline(never)]
fn t_options() {
    print_str("options");
    let mut o = black_box(Opts { k: 1, o: None, on: None, ob: None, op: None, r: Err(mkn(1)) });
    let (ao, aon, aop) = (ad(&o.o), ad(&o.on), ad(&o.op));
    o.o = Some(mk(1));
    o.on = Some(mkn(2));
    o.ob = Some(Box::new(mkn(3)));
    o.op = Some(Pair { s: mk(4), n: mkn(5) });
    o.r = Ok(mk(6));
    print_u32(o.o.map_or(0, |s| s.a) * 10000 + o.on.as_ref().map_or(0, |s| s.a) * 1000 + o.ob.as_ref().map_or(0, |s| s.a) * 100 + o.op.as_ref().map_or(0, |p| p.s.a + p.n.a) * 10 + o.r.as_ref().map_or(0, |s| s.a));
    o.o = None;
    o.on = None;
    o.ob = None;
    o.op = None;
    o.r = Err(mkn(7));
    print_bool(o.o.is_none() && o.on.is_none() && o.ob.is_none() && o.op.is_none() && o.r.is_err());
    o.o = Some(mk(2));
    o.on = Some(mkn(3));
    o.ob = Some(Box::new(mkn(4)));
    o.op = Some(Pair { s: mk(5), n: mkn(6) });
    o.o = Some(mk(3));
    o.on = Some(mkn(4));
    o.ob = Some(Box::new(mkn(5)));
    o.op = Some(Pair { s: mk(6), n: mkn(7) });
    print_u32(o.o.map_or(0, |s| s.a) * 10000 + o.on.as_ref().map_or(0, |s| s.a) * 1000 + o.ob.as_ref().map_or(0, |s| s.a) * 100 + o.op.as_ref().map_or(0, |p| p.s.a + p.n.a) * 10 + o.r.as_ref().map_or(0, |s| s.a));
    print_bool((ao, aon, aop) == (ad(&o.o), ad(&o.on), ad(&o.op)));
    let r = &mut o;
    r.on = r.on.take();
    r.op = None;
    r.op = r.on.take().map(|n| Pair { s: mk(n.a), n });
    print_u32(r.op.as_ref().map_or(0, |p| p.s.a * 10 + p.n.b) + if r.on.is_none() { 100 } else { 0 });
    let ro = &mut r.o;
    *ro = None;
    *ro = Some(mk(7));
    *ro = ro.map(|s| S { a: s.b, b: s.a });
    print_u32(ro.map_or(0, |s| s.a * 10 + s.b));
    let mut vo: Vec<Option<S>> = Vec::new();
    vo.push(None);
    vo.push(Some(mk(1)));
    vo.push(None);
    vo[0] = Some(mk(2));
    vo[1] = None;
    vo[2] = vo[0];
    vo.swap(0, 1);
    vo.reverse();
    print_u32(vo[0].map_or(9, |s| s.a) * 100 + vo[1].map_or(9, |s| s.a) * 10 + vo[2].map_or(9, |s| s.a));
    let mut vn: Vec<Option<N>> = Vec::new();
    vn.push(Some(mkn(1)));
    vn.push(None);
    vn.swap(0, 1);
    vn[0] = vn[1].take();
    vn[1] = Some(mkn(4));
    vn.sort_by_key(|n| n.as_ref().map_or(0, |n| 10 - n.a));
    print_u32(vn[0].as_ref().map_or(9, |s| s.a) * 10 + vn[1].as_ref().map_or(9, |s| s.a));
}

#[inline(never)]
fn t_late_locals(c: bool) {
    print_str("late_locals");
    let x: S;
    let y: N;
    if c {
        x = mk(1);
        y = mkn(2);
    } else {
        x = mk(3);
        y = mkn(4);
    }
    let px = ad(&x);
    print_u32(x.a * 10 + y.a);
    print_bool(px == ad(&x));
    let mut acc = 0;
    let mut cur: N;
    let mut held: Option<N> = None;
    for i in 0..4 {
        cur = mkn(i);
        let p = ad(&cur);
        if i % 2 == 0 {
            held = Some(cur);
        } else {
            acc += cur.a + if p != 0 { 100 } else { 0 };
        }
        if let Some(h) = held.take() {
            acc += h.a * 10;
        }
    }
    print_u32(acc);
    let mut z = mk(1);
    let pz = ad(&z);
    for i in 0..3 {
        let old = z;
        z = S { a: old.b + i, b: old.a };
    }
    print_u32(z.a * 10 + z.b);
    print_bool(pz == ad(&z));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_maybe_uninit();
    t_options();
    t_late_locals(black_box(true));
}
