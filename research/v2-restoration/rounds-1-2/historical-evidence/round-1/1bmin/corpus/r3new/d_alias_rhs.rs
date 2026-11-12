//! (d) a whole-value store whose right side reads the place being written: a field-wise
//! in-place store must not observe partially written fields; and no sharing after a store.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
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
#[derive(Clone, Copy)]
struct T2 {
    p: S,
    q: S,
}
struct TN {
    p: N,
    q: N,
}
struct O {
    k: u32,
    s: S,
    t: T2,
    tn: TN,
    pair: (S, S),
    npair: (N, N),
}
#[derive(Clone, Copy)]
enum E {
    A { x: u32, y: u32 },
    B { h: S, k: u32 },
    C,
}
struct WE {
    e: E,
    f: E,
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
fn mko() -> O {
    black_box(O {
        k: 1,
        s: mk(1),
        t: T2 { p: mk(3), q: mk(5) },
        tn: TN { p: mkn(3), q: mkn(5) },
        pair: (mk(7), mk(9)),
        npair: (mkn(7), mkn(9)),
    })
}
#[inline(never)]
fn peek_a(p: *const O) -> u32 {
    unsafe { (*p).s.a }
}
#[inline(never)]
fn peek_b(p: *const O) -> u32 {
    unsafe { (*p).s.b }
}
#[inline(never)]
fn flip(s: &S) -> S {
    S { a: s.b, b: s.a }
}
#[inline(always)]
fn flip_inl(s: &S) -> S {
    S { a: s.b, b: s.a }
}
#[inline(never)]
fn flip_val(s: S) -> S {
    S { a: s.b, b: s.a }
}
#[inline(never)]
fn flipn(s: N) -> N {
    N { a: s.b, b: s.a }
}
fn cs(s: &S) -> u32 {
    s.a * 10 + s.b
}
fn cn(s: &N) -> u32 {
    s.a * 10 + s.b
}
fn ce(e: &E) -> u32 {
    match e {
        E::A { x, y } => 1000 + x * 10 + y,
        E::B { h, k } => 2000 + h.a * 100 + h.b * 10 + k,
        E::C => 3000,
    }
}

#[inline(never)]
fn t_scalars() {
    print_str("scalars");
    let mut o = mko();
    o.s = S { a: o.s.b, b: o.s.a };
    print_u32(cs(&o.s));
    {
        let r = &mut o;
        r.s = S { a: r.s.b, b: r.s.a };
        print_u32(cs(&r.s));
        let rs = &mut r.s;
        *rs = S { a: rs.b, b: rs.a };
        print_u32(cs(rs));
        *rs = S { a: rs.b + rs.a, b: rs.a };
        print_u32(cs(rs));
        *rs = S { b: rs.a, a: rs.b };
        print_u32(cs(rs));
    }
    let p: *mut O = &mut o;
    unsafe {
        (*p).s = S { a: peek_b(p), b: peek_a(p) };
        print_u32(cs(&(*p).s));
        (*p).s = S { a: 9, b: peek_a(p) };
        print_u32(cs(&(*p).s));
        (*p).s = S { a: peek_b(p), b: 4 };
        print_u32(cs(&(*p).s));
        (*p).s = S { a: (*p).s.b, b: (*p).s.a };
        print_u32(cs(&(*p).s));
        *(&raw mut (*p).s) = S { a: (*p).s.b, b: (*p).s.a };
        print_u32(cs(&(*p).s));
    }
    o.s = flip(&o.s);
    print_u32(cs(&o.s));
    o.s = flip_inl(&o.s);
    print_u32(cs(&o.s));
    o.s = flip_val(o.s);
    print_u32(cs(&o.s));
    o.s = o.s;
    print_u32(cs(&o.s));
    let c = o.s;
    o.s = S { a: 0, b: 0 };
    o.s = c;
    print_u32(cs(&o.s) + cs(&c));
}

#[inline(never)]
fn t_aggs() {
    print_str("aggs");
    let mut o = mko();
    o.t = T2 { p: o.t.q, q: o.t.p };
    print_u32(cs(&o.t.p) * 100 + cs(&o.t.q));
    o.tn = TN { p: o.tn.q, q: o.tn.p };
    print_u32(cn(&o.tn.p) * 100 + cn(&o.tn.q));
    o.pair = (o.pair.1, o.pair.0);
    print_u32(cs(&o.pair.0) * 100 + cs(&o.pair.1));
    o.npair = (o.npair.1, o.npair.0);
    print_u32(cn(&o.npair.0) * 100 + cn(&o.npair.1));
    {
        let r = &mut o;
        r.t = T2 { p: r.t.q, q: r.t.p };
        r.pair = (r.pair.1, r.pair.0);
        let rt = &mut r.t;
        *rt = T2 { p: S { a: rt.q.b, b: rt.p.a }, q: rt.p };
        print_u32(cs(&rt.p) * 100 + cs(&rt.q));
        let rp = &mut r.pair;
        *rp = (rp.1, rp.0);
        print_u32(cs(&rp.0) * 100 + cs(&rp.1));
    }
    o.tn.p = flipn(o.tn.p);
    o.tn.q = flipn(o.tn.q);
    print_u32(cn(&o.tn.p) * 100 + cn(&o.tn.q));
    o.tn = TN { p: flipn(o.tn.q), q: o.tn.p };
    print_u32(cn(&o.tn.p) * 100 + cn(&o.tn.q));
    let mut pair = black_box((mk(1), mk(3)));
    let keep = &raw const pair.0;
    pair = (pair.1, pair.0);
    print_u32(cs(&pair.0) * 100 + cs(&pair.1));
    print_u32(unsafe { (*keep).a });
    let mut np = black_box((mkn(1), mkn(3)));
    np = (np.1, np.0);
    print_u32(cn(&np.0) * 100 + cn(&np.1));
    let mut arr = black_box([mkn(1), mkn(3)]);
    let [x, y] = arr;
    arr = [y, x];
    print_u32(cn(&arr[0]) * 100 + cn(&arr[1]));
    let mut b = Box::new(black_box(T2 { p: mk(1), q: mk(3) }));
    *b = T2 { p: b.q, q: b.p };
    print_u32(cs(&b.p) * 100 + cs(&b.q));
    let mut v = vec![black_box(T2 { p: mk(1), q: mk(3) }), black_box(T2 { p: mk(5), q: mk(7) })];
    v[0] = T2 { p: v[0].q, q: v[1].p };
    v[1] = T2 { p: v[0].q, q: v[1].p };
    print_u32(cs(&v[0].p) * 100 + cs(&v[0].q));
    print_u32(cs(&v[1].p) * 100 + cs(&v[1].q));
}

#[inline(never)]
fn t_enums() {
    print_str("enums");
    let mut w = black_box(WE { e: E::A { x: 1, y: 2 }, f: E::B { h: mk(3), k: 5 } });
    w.e = match w.e {
        E::A { x, y } => E::A { x: y, y: x },
        other => other,
    };
    print_u32(ce(&w.e));
    w.f = match w.f {
        E::B { h, k } => E::B { h: S { a: h.b, b: h.a }, k: k + 1 },
        other => other,
    };
    print_u32(ce(&w.f));
    let e = &mut w.f;
    if let E::B { h, k } = *e {
        *e = E::B { h: S { a: k, b: h.a }, k: h.b };
    }
    print_u32(ce(e));
    *e = match *e {
        E::B { h, .. } => E::A { x: h.b, y: h.a },
        _ => E::C,
    };
    print_u32(ce(e));
    *e = match *e {
        E::A { x, y } => E::B { h: S { a: y, b: x }, k: x + y },
        _ => E::C,
    };
    print_u32(ce(e));
    w.e = w.f;
    if let E::B { h, .. } = &mut w.e {
        h.a += 50;
    }
    print_u32(ce(&w.e));
    print_u32(ce(&w.f));
    w.f = E::C;
    print_u32(ce(&w.e) + ce(&w.f));
    w.f = w.e;
    w.e = E::A { x: 1, y: 1 };
    print_u32(ce(&w.e) + ce(&w.f));
}

#[inline(never)]
fn t_no_sharing() {
    print_str("no_sharing");
    let mut o = mko();
    o.t.p = o.t.q;
    o.t.p.a += 100;
    print_u32(o.t.q.a);
    o.pair.0 = o.s;
    o.s.a += 100;
    print_u32(o.pair.0.a);
    let mut o2 = mko();
    o2.t = o.t;
    o2.t.q.b += 100;
    print_u32(o.t.q.b);
    {
        let (r1, r2) = (&mut o, &mut o2);
        r1.t = r2.t;
        r2.t.p.a = 77;
        print_u32(r1.t.p.a);
        *r1 = mko();
        r1.pair = r2.pair;
        r2.pair.1.b = 66;
        print_u32(r1.pair.1.b);
    }
    let mut v: Vec<T2> = vec![o.t, o2.t];
    v[0] = v[1];
    v[1].p.a = 55;
    print_u32(v[0].p.a);
    let t = v[1];
    v[1] = T2 { p: mk(0), q: mk(0) };
    print_u32(t.p.a);
    let mut arr = [o.t; 3];
    arr[1].p.a = 44;
    arr[2] = arr[1];
    arr[1].p.a = 33;
    print_u32(arr[0].p.a * 10000 + arr[1].p.a * 100 + arr[2].p.a);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_scalars();
    t_aggs();
    t_enums();
    t_no_sharing();
}
