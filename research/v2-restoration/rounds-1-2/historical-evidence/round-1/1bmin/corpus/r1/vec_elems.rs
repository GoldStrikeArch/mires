//! (c) Vec and array elements of aggregates: reassignment in place, growth, removal.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;
use core::mem;
use core::ptr;
use prelude::*;

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}
#[derive(Clone, Copy)]
struct Q {
    p: P,
    z: u32,
}
struct D {
    v: u32,
}

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

fn q(x: u32, z: u32) -> Q {
    Q { p: P { x, y: x + 1 }, z }
}

#[inline(never)]
fn e_vec() {
    let mut v: Vec<Q> = Vec::new();
    for i in 0..3 {
        v.push(q(i, i * 2));
    }
    let at1 = a(&v[1]);
    let atp = a(&v[1].p);
    let atz = a(&v[1].z);
    v[1] = q(10, 12);
    print_bool(a(&v[1]) == at1);
    print_bool(a(&v[1].p) == atp);
    print_bool(a(&v[1].z) == atz);
    print_bool(atp == at1);
    print_bool(atz - at1 == mem::offset_of!(Q, z));
    print_u32(v[1].p.x + v[1].z);
    let raw1 = &raw const v[1];
    v[1] = q(13, 14);
    print_bool(ptr::eq(raw1, &raw const v[1]));
    print_bool(raw1 as usize == a(&v[1]));
    print_bool(ptr::eq(raw1, &v[1]));
    print_bool(ptr::eq(raw1, v.as_ptr().wrapping_add(1)));
    print_u32(unsafe { (*raw1).p.x });
    // copy out, mutate copy
    let mut c = v[1];
    c.p.x += 100;
    print_u32(v[1].p.x);
    print_u32(c.p.x);
    // growth: values only
    let before: u32 = v.iter().map(|q| q.p.x + q.z).sum();
    for i in 0..20 {
        v.push(q(i, 0));
    }
    print_u32(before);
    print_usize(v.len());
    print_u32(v[1].p.x + v[22].p.x);
    // pop then push: the popped must not alias the new element
    let mut popped = v.pop().unwrap();
    v.push(q(777, 0));
    popped.p.x += 1;
    print_u32(popped.p.x);
    print_u32(v[v.len() - 1].p.x);
    let r = v.swap_remove(0);
    print_u32(r.p.x + r.z);
    print_u32(v[0].p.x);
    let r = v.remove(1);
    print_u32(r.p.x);
    v.insert(1, q(55, 1));
    print_u32(v[1].p.x + v[2].p.x);
    v.swap(0, 2);
    print_u32(v[0].p.x * 1000 + v[2].p.x);
    v.truncate(4);
    v.reverse();
    print_u32(v[0].p.x + v[3].p.x);
    for e in v.iter_mut() {
        *e = Q { p: P { x: e.p.x + 1, y: 0 }, z: 9 };
    }
    print_u32(v.iter().map(|q| q.p.x + q.z).sum::<u32>());
    // non-Copy elements
    let mut d: Vec<D> = vec![D { v: 1 }, D { v: 2 }, D { v: 3 }];
    let atd = a(&d[2]);
    d[2] = D { v: 30 };
    print_bool(a(&d[2]) == atd);
    let mut taken = d.pop().unwrap();
    d.push(D { v: 31 });
    taken.v += 1;
    print_u32(taken.v);
    print_u32(d[2].v);
    let mut drained: Vec<D> = d.drain(..2).collect();
    drained[0].v += 100;
    d.push(D { v: 5 });
    print_u32(drained[0].v + drained[1].v + d[0].v + d[1].v);
    let last = d.last_mut().unwrap();
    *last = D { v: 6 };
    print_u32(d[1].v);
    // Vec<Vec<Q>>: inner vec reassigned whole
    let mut vv: Vec<Vec<Q>> = vec![vec![q(1, 1)], vec![q(2, 2), q(3, 3)]];
    let ati = a(&vv[1]);
    vv[1] = vec![q(4, 4)];
    print_bool(a(&vv[1]) == ati);
    print_usize(vv[0].len() + vv[1].len());
    print_u32(vv[1][0].p.x);
    vv[0].push(q(5, 5));
    print_u32(vv[0][1].z + vv[1][0].z);
}

#[inline(never)]
fn e_array() {
    let mut arr = black_box([q(1, 1), q(2, 2), q(3, 3)]);
    let at1 = a(&arr[1]);
    arr[1] = q(20, 20);
    print_bool(a(&arr[1]) == at1);
    let mut c = arr[1];
    c.p.x += 1;
    print_u32(arr[1].p.x);
    print_u32(c.p.x);
    let b = arr;
    arr[0].p.x = 99;
    print_u32(b[0].p.x);
    print_u32(arr[0].p.x);
    let at0 = a(&arr[0]);
    arr = black_box([q(7, 7), q(8, 8), q(9, 9)]);
    print_bool(a(&arr[0]) == at0);
    print_u32(arr[0].p.x + b[2].p.x);
    arr.swap(0, 2);
    arr.reverse();
    print_u32(arr[0].p.x * 100 + arr[1].p.x * 10 + arr[2].p.x);
    let mut ds = [D { v: 1 }, D { v: 2 }];
    let old = mem::replace(&mut ds[0], D { v: 10 });
    print_u32(old.v + ds[0].v);
    let [first, second] = ds;
    print_u32(first.v * 10 + second.v);
    // array field inside a struct, reassigned whole and per element
    struct Holder {
        n: u32,
        arr: [Q; 2],
    }
    let mut h = black_box(Holder { n: 1, arr: [q(1, 1), q(2, 2)] });
    let ah = a(&h.arr);
    let ah1 = a(&h.arr[1]);
    h.arr = [q(3, 3), q(4, 4)];
    print_bool(a(&h.arr) == ah);
    print_bool(a(&h.arr[1]) == ah1);
    print_bool(ah == a(&h));
    h.arr[1] = q(5, 5);
    print_bool(a(&h.arr[1]) == ah1);
    print_u32(h.arr[0].p.x + h.arr[1].p.x + h.n);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_vec();
    e_array();
}
