//! (a) pointer-valued places: moving references / raw pointers / Option<&T> / Box between
//! places must rebind and never write into the pointee.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;
use core::hint::black_box;
use core::mem;
use core::ptr;
use prelude::*;

// `slice::rotate_left` is refused by the backend in both arms (core/src/slice/rotate.rs casts a
// `MaybeUninit<([usize; 32], [T; 0])>` pointer), so rotation is spelled with swaps.
fn rot_left<T>(s: &mut [T], k: usize) {
    for _ in 0..k {
        for i in 1..s.len() {
            s.swap(i - 1, i);
        }
    }
}
fn rot_right<T>(s: &mut [T], k: usize) {
    for _ in 0..k {
        for i in (1..s.len()).rev() {
            s.swap(i - 1, i);
        }
    }
}

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}
#[inline(never)]
fn mk(x: u32) -> P {
    black_box(P { x, y: x + 100 })
}
fn sum(ps: &[&P]) -> u32 {
    let mut s = 0;
    for (i, p) in ps.iter().enumerate() {
        s = s * 10 + p.x + (p.y - 100 - p.x) * 1000 + i as u32 * 0;
    }
    s
}
fn vals(a: &P, b: &P, c: &P, d: &P) -> u32 {
    a.x * 1000 + b.x * 100 + c.x * 10 + d.x + (a.y + b.y + c.y + d.y - 400 - a.x - b.x - c.x - d.x) * 100000
}

#[inline(never)]
fn set_ref<'a>(rr: &mut &'a P, n: &'a P) {
    *rr = n;
}
#[inline(never)]
fn set_opt<'a>(rr: &mut Option<&'a P>, n: Option<&'a P>) {
    *rr = n;
}
#[inline(never)]
fn set_raw(rr: &mut *const P, n: *const P) {
    *rr = n;
}

#[inline(never)]
fn t_vec_refs() {
    print_str("vec_refs");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut v: Vec<&P> = vec![&p1, &p2, &p3, &p4];
    v.swap(0, 3);
    print_u32(sum(&v));
    print_u32(vals(&p1, &p2, &p3, &p4));
    v.reverse();
    print_u32(sum(&v));
    print_u32(vals(&p1, &p2, &p3, &p4));
    v.sort_by_key(|r| 10 - r.x);
    print_u32(sum(&v));
    rot_left(&mut v, 1);
    print_u32(sum(&v));
    rot_right(&mut v, 2);
    print_u32(sum(&v));
    print_u32(vals(&p1, &p2, &p3, &p4));
    let old = mem::replace(&mut v[0], &p4);
    print_u32(old.x * 10 + v[0].x);
    print_bool(ptr::eq(v[0], &p4));
    print_u32(vals(&p1, &p2, &p3, &p4));
    v.insert(1, &p1);
    let removed = v.remove(3);
    print_u32(removed.x);
    print_u32(sum(&v));
    v.retain(|r| r.x != 4);
    print_u32(sum(&v));
    v.truncate(1);
    v.push(&p3);
    v.push(&p2);
    print_u32(sum(&v));
    v.copy_within(1..3, 0);
    print_u32(sum(&v));
    v.sort_unstable_by_key(|r| r.x);
    print_u32(sum(&v));
    v.dedup_by_key(|r| r.x);
    print_u32(sum(&v));
    print_u32(vals(&p1, &p2, &p3, &p4));
    let mut hits = 0;
    for r in v.iter() {
        if ptr::eq(*r, &p1) || ptr::eq(*r, &p2) || ptr::eq(*r, &p3) || ptr::eq(*r, &p4) {
            hits += 1;
        }
    }
    print_u32(hits);
}

#[inline(never)]
fn t_arr_refs() {
    print_str("arr_refs");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut a: [&P; 4] = black_box([&p1, &p2, &p3, &p4]);
    a.swap(1, 2);
    a.reverse();
    print_u32(sum(&a));
    rot_left(&mut a, 3);
    print_u32(sum(&a));
    a.sort_by_key(|r| r.x);
    print_u32(sum(&a));
    let old = mem::replace(&mut a[2], &p1);
    print_u32(old.x);
    a = [a[3], a[2], a[1], a[0]];
    print_u32(sum(&a));
    a[0] = a[1];
    print_u32(sum(&a));
    unsafe { ptr::write(&mut a[3], &p2) };
    unsafe { ptr::swap(&mut a[0], &mut a[3]) };
    print_u32(sum(&a));
    let pa = a.as_mut_ptr();
    unsafe { ptr::copy(pa, pa.add(1), 3) };
    print_u32(sum(&a));
    unsafe { ptr::copy_nonoverlapping(pa.add(2), pa, 1) };
    print_u32(sum(&a));
    print_u32(vals(&p1, &p2, &p3, &p4));
    print_bool(ptr::eq(a[0], a[2]));
}

#[inline(never)]
fn t_raws() {
    print_str("raws");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut v: Vec<*const P> = vec![&p1, &p2, &p3, &p4];
    v.swap(0, 2);
    v.reverse();
    v.sort_by_key(|p| unsafe { (**p).y } ^ 1);
    rot_left(&mut v, 1);
    let mut s = 0;
    for p in &v {
        s = s * 10 + unsafe { (**p).x };
    }
    print_u32(s);
    let old = mem::replace(&mut v[1], &p1 as *const P);
    print_u32(unsafe { (*old).x });
    print_bool(ptr::eq(v[1], &p1));
    let mut r: *const P = &p1;
    set_raw(&mut r, &p3);
    print_u32(unsafe { (*r).x });
    print_u32(vals(&p1, &p2, &p3, &p4));
    let mut arr: [*const P; 2] = [&p1, &p2];
    arr.swap(0, 1);
    arr = [arr[0], arr[0]];
    print_bool(ptr::eq(arr[0], &p2) && ptr::eq(arr[1], &p2));
    print_u32(vals(&p1, &p2, &p3, &p4));
}

#[inline(never)]
fn t_opts() {
    print_str("opts");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut v: Vec<Option<&P>> = vec![Some(&p1), None, Some(&p3), Some(&p4)];
    v.swap(0, 1);
    v.swap(2, 3);
    v.reverse();
    let code = |v: &Vec<Option<&P>>| {
        let mut s = 0;
        for o in v {
            s = s * 10 + o.map_or(9, |p| p.x);
        }
        s
    };
    print_u32(code(&v));
    let t = v[0].take();
    print_u32(t.map_or(0, |p| p.x));
    let old = mem::replace(&mut v[1], Some(&p2));
    print_u32(old.map_or(0, |p| p.x));
    v[2] = Some(&p2);
    v[3] = v[1];
    print_u32(code(&v));
    v.sort_by_key(|o| o.map_or(0, |p| 10 - p.x));
    print_u32(code(&v));
    let mut o = black_box(Some(&p1));
    set_opt(&mut o, Some(&p4));
    print_u32(o.map_or(0, |p| p.x));
    set_opt(&mut o, None);
    print_bool(o.is_none());
    set_opt(&mut o, Some(&p2));
    print_u32(o.map_or(0, |p| p.x));
    print_u32(vals(&p1, &p2, &p3, &p4));
}

#[inline(never)]
fn t_boxes() {
    print_str("boxes");
    let mut v: Vec<Box<P>> = vec![Box::new(mk(1)), Box::new(mk(2)), Box::new(mk(3))];
    let first = &*v[0] as *const P;
    let third = &*v[2] as *const P;
    v.swap(0, 2);
    print_bool(ptr::eq(&*v[2], first) && ptr::eq(&*v[0], third));
    v.reverse();
    print_bool(ptr::eq(&*v[0], first));
    v.sort_by_key(|b| 10 - b.x);
    print_u32(v[0].x * 100 + v[1].x * 10 + v[2].x);
    print_bool(ptr::eq(&*v[2], first));
    let old = mem::replace(&mut v[2], Box::new(mk(7)));
    print_u32(old.x * 10 + v[2].x);
    print_bool(ptr::eq(&*old, first));
    print_bool(!ptr::eq(&*v[2], first));
    rot_left(&mut v, 1);
    print_u32(v[0].x * 100 + v[1].x * 10 + v[2].x);
    print_u32(unsafe { (*third).x });
    *v[0] = mk(8);
    print_u32(v[0].x * 100 + v[1].x * 10 + v[2].x + old.y);
}

struct Holder<'a> {
    r: &'a P,
    o: Option<&'a P>,
    p: *const P,
    n: u32,
}

#[inline(never)]
fn t_fields() {
    print_str("fields");
    let (p1, p2, p3, p4) = (mk(1), mk(2), mk(3), mk(4));
    let mut h = black_box(Holder { r: &p1, o: Some(&p2), p: &p3, n: 0 });
    h.r = &p4;
    h.o = Some(&p1);
    h.p = &p2;
    print_u32(h.r.x * 100 + h.o.map_or(0, |p| p.x) * 10 + unsafe { (*h.p).x });
    print_u32(vals(&p1, &p2, &p3, &p4));
    let hm = &mut h;
    *hm = Holder { r: &p3, o: None, p: &p4, n: 1 };
    print_u32(hm.r.x * 100 + hm.o.map_or(0, |p| p.x) * 10 + unsafe { (*hm.p).x } + hm.n * 1000);
    print_u32(vals(&p1, &p2, &p3, &p4));
    let mut r: &P = &p1;
    set_ref(&mut r, &p2);
    print_u32(r.x * 10 + p1.x);
    let mut r2: &P = &p3;
    mem::swap(&mut r, &mut r2);
    print_u32(r.x * 10 + r2.x);
    unsafe { ptr::swap(&mut r, &mut r2) };
    print_u32(r.x * 10 + r2.x);
    print_u32(vals(&p1, &p2, &p3, &p4));
    let c = Cell::new(&p1);
    c.set(&p2);
    let was = c.replace(&p3);
    print_u32(was.x * 10 + c.get().x);
    let c2 = Cell::new(&p4);
    c.swap(&c2);
    print_u32(c.get().x * 10 + c2.get().x);
    let co: Cell<Option<&P>> = Cell::new(Some(&p1));
    co.set(Some(&p2));
    let t = co.take();
    print_u32(t.map_or(0, |p| p.x) + if co.get().is_none() { 50 } else { 0 });
    print_u32(vals(&p1, &p2, &p3, &p4));
}

trait Shape {
    fn area(&self) -> u32;
}
impl Shape for P {
    fn area(&self) -> u32 {
        self.x * 2
    }
}
struct Q(u32);
impl Shape for Q {
    fn area(&self) -> u32 {
        self.0 + 1
    }
}

#[inline(never)]
fn t_fat() {
    print_str("fat");
    let (p1, p2) = (mk(1), mk(2));
    let q = Q(40);
    let a1 = [1u32, 2, 3];
    let a2 = [7u32, 8];
    let mut slices: Vec<&[u32]> = vec![&a1, &a2, &a1[1..]];
    slices.swap(0, 1);
    slices.reverse();
    slices.sort_by_key(|s| s.len());
    print_u32(slices[0][0] * 100 + slices[1][0] * 10 + slices[2][0]);
    print_u32(a1[0] * 100 + a1[1] * 10 + a1[2] + a2[0] * 10000 + a2[1] * 1000);
    let mut dyns: Vec<&dyn Shape> = vec![&p1, &q, &p2];
    dyns.swap(0, 1);
    dyns.reverse();
    print_u32(dyns[0].area() * 10000 + dyns[1].area() * 100 + dyns[2].area());
    dyns.sort_by_key(|d| d.area());
    print_u32(dyns[0].area() * 10000 + dyns[1].area() * 100 + dyns[2].area());
    print_u32(p1.x * 10 + p2.x + q.0 * 100);
    let mut strs: Vec<&str> = vec!["bb", "a", "ccc"];
    strs.sort();
    strs.swap(0, 2);
    print_str(strs[0]);
    print_str(strs[2]);
    let mut bd: Vec<Box<dyn Shape>> = vec![Box::new(mk(5)), Box::new(Q(1)), Box::new(mk(3))];
    bd.swap(0, 2);
    bd.sort_by_key(|d| d.area());
    print_u32(bd[0].area() * 10000 + bd[1].area() * 100 + bd[2].area());
    let mut pairs: Vec<(&P, u32)> = vec![(&p1, 1), (&p2, 2)];
    pairs.swap(0, 1);
    pairs[1] = (pairs[0].0, 9);
    print_u32(pairs[0].0.x * 100 + pairs[1].0.x * 10 + pairs[1].1);
    print_u32(p1.x * 10 + p2.x + p1.y + p2.y);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    t_vec_refs();
    t_arr_refs();
    t_raws();
    t_opts();
    t_boxes();
    t_fields();
    t_fat();
}
