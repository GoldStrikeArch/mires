//! (f) assigning an aggregate with Drop fields over an existing one: the old fields are dropped
//! exactly once, in native order, after the new value is built.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem;
use core::ptr;
use prelude::*;

struct D(u32);
impl Drop for D {
    fn drop(&mut self) {
        print_u32(self.0)
    }
}
struct Two {
    a: D,
    b: D,
}
struct Plain {
    x: u32,
    y: u32,
}
struct Mixed {
    p: Plain,
    d: D,
    two: Two,
}
struct WithDrop {
    k: u32,
    d: D,
}
impl Drop for WithDrop {
    fn drop(&mut self) {
        print_u32(9000 + self.k)
    }
}
enum E {
    One(D),
    Pair { a: D, b: D },
    Empty,
}
struct Holder {
    two: Two,
    m: Mixed,
    o: Option<Two>,
    e: E,
    arr: [D; 2],
    w: WithDrop,
}
#[inline(never)]
fn mkd(n: u32) -> D {
    print_u32(100000 + n);
    black_box(D(n))
}
fn ad<T>(r: &T) -> usize {
    r as *const T as usize
}
#[inline(never)]
fn holder() -> Holder {
    Holder {
        two: Two { a: D(1), b: D(2) },
        m: Mixed { p: Plain { x: 1, y: 2 }, d: D(3), two: Two { a: D(4), b: D(5) } },
        o: Some(Two { a: D(6), b: D(7) }),
        e: E::Pair { a: D(8), b: D(9) },
        arr: [D(10), D(11)],
        w: WithDrop { k: 12, d: D(13) },
    }
}

#[inline(never)]
fn t_fields() {
    print_str("fields");
    let mut h = holder();
    let at = (ad(&h.two), ad(&h.m.two), ad(&h.w), ad(&h.m.p));
    print_str("two");
    h.two = Two { a: mkd(21), b: mkd(22) };
    print_str("mixed");
    h.m = Mixed { p: Plain { x: 3, y: 4 }, d: mkd(23), two: Two { a: mkd(24), b: mkd(25) } };
    print_str("opt");
    h.o = Some(Two { a: mkd(26), b: mkd(27) });
    h.o = None;
    h.o = Some(Two { a: mkd(28), b: mkd(29) });
    print_str("enum");
    h.e = E::One(mkd(30));
    h.e = E::Pair { a: mkd(31), b: mkd(32) };
    h.e = E::Empty;
    h.e = E::One(mkd(33));
    print_str("arr");
    h.arr = [mkd(34), mkd(35)];
    h.arr[0] = mkd(36);
    print_str("withdrop");
    h.w = WithDrop { k: 37, d: mkd(38) };
    print_bool(at == (ad(&h.two), ad(&h.m.two), ad(&h.w), ad(&h.m.p)));
    print_u32(h.m.p.x * 10 + h.m.p.y + h.two.a.0 + h.m.two.b.0 + h.w.k);
    print_str("end");
}

#[inline(never)]
fn set_two(r: &mut Two, v: Two) {
    *r = v;
}

#[inline(never)]
fn t_through() {
    print_str("through");
    let mut h = holder();
    {
        let r = &mut h.two;
        *r = Two { a: mkd(41), b: mkd(42) };
        print_str("callee");
        set_two(r, Two { a: mkd(43), b: mkd(44) });
    }
    print_str("replace");
    let old = mem::replace(&mut h.two, Two { a: mkd(45), b: mkd(46) });
    print_u32(old.a.0 * 100 + h.two.a.0);
    drop(old);
    print_str("swap");
    let mut other = Two { a: mkd(47), b: mkd(48) };
    mem::swap(&mut h.two, &mut other);
    print_u32(other.a.0 * 100 + h.two.a.0);
    drop(other);
    print_str("write");
    let p: *mut Two = &mut h.two;
    unsafe {
        ptr::drop_in_place(p);
        ptr::write(p, Two { a: mkd(49), b: mkd(50) });
    }
    print_str("moved");
    let taken = h.two;
    h.two = Two { a: mkd(51), b: mkd(52) };
    print_u32(taken.a.0 * 100 + h.two.a.0);
    drop(taken);
    print_str("partial");
    let a_only = h.m.two.a;
    h.m.two.a = mkd(53);
    drop(a_only);
    h.m.two = Two { a: mkd(54), b: mkd(55) };
    print_str("local");
    let mut t = Two { a: mkd(56), b: mkd(57) };
    let at = ad(&t);
    t = Two { a: mkd(58), b: mkd(59) };
    print_bool(at == ad(&t));
    print_u32(t.b.0);
    print_str("end");
}

#[inline(never)]
fn t_heap() {
    print_str("heap");
    let mut v: Vec<Two> = vec![Two { a: D(61), b: D(62) }, Two { a: D(63), b: D(64) }];
    v[0] = Two { a: mkd(65), b: mkd(66) };
    print_str("swap");
    v.swap(0, 1);
    print_u32(v[0].a.0 * 100 + v[1].a.0);
    print_str("truncate");
    v.truncate(1);
    v.push(Two { a: mkd(67), b: mkd(68) });
    print_str("remove");
    let r = v.remove(0);
    print_u32(r.b.0);
    drop(r);
    print_str("box");
    let mut b = Box::new(Two { a: D(69), b: D(70) });
    *b = Two { a: mkd(71), b: mkd(72) };
    let inner = *b;
    print_u32(inner.a.0);
    print_str("end");
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_fields();
    t_through();
    t_heap();
}
