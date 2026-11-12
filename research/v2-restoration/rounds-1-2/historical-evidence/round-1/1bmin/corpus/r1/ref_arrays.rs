//! (h) arrays and vectors of references and raw pointers: copies must not touch referents.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;
use core::mem;
use core::ptr;
use prelude::*;

struct P {
    v: Cell<u32>,
    w: u32,
}

fn p(w: u32) -> P {
    P { v: Cell::new(w), w }
}

#[inline(never)]
fn e_refs() {
    let p0 = p(10);
    let p1 = p(20);
    let p2 = p(30);
    let p3 = p(40);
    let a3: [&P; 3] = [&p0, &p1, &p2];
    let b3 = a3;
    b3[0].v.set(100);
    print_u32(a3[0].v.get());
    print_bool(ptr::eq(a3[1], b3[1]));
    let mut m = a3;
    m.swap(0, 2);
    m.reverse();
    print_u32(m[0].w * 100 + m[1].w * 10 + m[2].w);
    print_u32(p0.w + p1.w + p2.w);
    print_u32(p0.v.get() + p1.v.get() + p2.v.get());
    print_bool(ptr::eq(m[2], &p0));
    print_bool(ptr::eq(a3[0], &p0));
    let mut v: Vec<&P> = vec![&p0, &p1, &p2];
    v.insert(0, &p3);
    v.remove(2);
    v.swap(0, 1);
    v.push(&p2);
    print_u32(v.iter().fold(0, |acc, p| acc * 100 + p.w));
    print_u32(p0.w * 1000 + p1.w * 100 + p2.w * 10 + p3.w);
    print_u32(p0.v.get() + p3.v.get());
    print_bool(ptr::eq(v[0], &p2));
    print_bool(ptr::eq(v[1], &p0));
    print_bool(ptr::eq(v[2], &p3));
    v[1].v.set(7);
    print_u32(p0.v.get());
    // Vec<*const P>: the pointer records must survive copies with their identities
    let mut rp: Vec<*const P> = vec![&p0 as *const P, &p1, &p2];
    rp.insert(1, &p3);
    rp.remove(0);
    rp.swap(0, 2);
    print_bool(ptr::eq(rp[0], &p2));
    print_bool(ptr::eq(rp[1], &p1));
    print_bool(ptr::eq(rp[2], &p3));
    print_bool(ptr::eq(&p0 as *const P, &p0));
    print_bool(ptr::eq(&p3 as *const P, &p3));
    print_u32(unsafe { (*rp[0]).w + (*rp[2]).w });
    // [&mut P; 2]: swap the references, mutate through them
    let mut q0 = p(1);
    let mut q1 = p(2);
    {
        let mut mm: [&mut P; 2] = [&mut q0, &mut q1];
        mm.swap(0, 1);
        mm[0].w += 10;
        mm[1].w += 20;
        print_u32(mm[0].w * 100 + mm[1].w);
    }
    print_u32(q0.w * 100 + q1.w);
    // Option<&P> arrays copied and swapped, two arrays of references swapped whole
    let mut oa: [Option<&P>; 2] = [Some(&p0), None];
    let ob = oa;
    oa.swap(0, 1);
    print_bool(oa[1].map(|p| p.w) == Some(10));
    print_bool(oa[0].is_none());
    print_bool(ob[0].is_some());
    let mut x = [&p0, &p1];
    let mut y = [&p2, &p3];
    mem::swap(&mut x, &mut y);
    print_u32(x[0].w + y[0].w * 10);
    print_bool(ptr::eq(x[1], &p3));
    print_u32(p0.w + p2.w);
    // tuples of references copied
    let t: (&P, &P) = (&p0, &p1);
    let t2 = t;
    print_bool(ptr::eq(t.0, t2.0) && ptr::eq(t.1, &p1));
    print_u32(p1.v.get());
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_refs();
}
