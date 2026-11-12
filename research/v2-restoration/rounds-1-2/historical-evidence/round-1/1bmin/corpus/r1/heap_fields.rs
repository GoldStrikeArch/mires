//! (d) a struct holding a Vec and a Box reassigned whole: drops exactly once, new value owned once.
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
use prelude::*;

static mut DROPS: u32 = 0;
static mut LOG: u64 = 0;
fn drops() -> u32 {
    unsafe { DROPS }
}
fn log() -> u64 {
    unsafe { LOG }
}
fn reset_log() {
    unsafe { LOG = 0 }
}

struct Tag(u32);
impl Drop for Tag {
    fn drop(&mut self) {
        unsafe {
            DROPS += 1;
            LOG = LOG.wrapping_mul(10).wrapping_add(self.0 as u64);
        }
    }
}

#[derive(Clone, Copy)]
struct H {
    head: u32,
    tail: u32,
}
struct S {
    n: u32,
    v: Vec<Tag>,
    b: Box<Tag>,
    h: H,
}
struct O {
    k: u32,
    s: S,
}

fn a<T>(r: &T) -> usize {
    r as *const T as usize
}

#[inline(never)]
fn mk(n: u32) -> S {
    black_box(S { n, v: vec![Tag(n), Tag(n + 1)], b: Box::new(Tag(n + 2)), h: H { head: n, tail: n } })
}

#[inline(never)]
fn e_whole() {
    let mut o = black_box(O { k: 1, s: mk(1) });
    let at = a(&o.s);
    let atv = a(&o.s.v);
    let ath = a(&o.s.h);
    let atn = a(&o.s.n);
    o.s = mk(4);
    print_u32(drops());
    print_u64(log());
    print_bool(a(&o.s) == at);
    print_bool(a(&o.s.v) == atv);
    print_bool(a(&o.s.h) == ath);
    print_bool(a(&o.s.n) == atn);
    print_usize(o.s.v.len());
    print_u32(o.s.v[1].0);
    print_u32(o.s.b.0);
    print_u32(o.s.h.head);
    o.s.v.push(Tag(9));
    print_usize(o.s.v.len());
    print_u32(drops());
    reset_log();
    // move out, reassign, mutate moved
    let mut x = o.s;
    o.s = mk(7);
    print_u32(drops());
    x.v.push(Tag(8));
    print_usize(x.v.len());
    print_usize(o.s.v.len());
    print_u32(x.v[3].0 + o.s.v[0].0);
    print_u32(x.b.0 + o.s.b.0);
    drop(x);
    print_u32(drops());
    print_u64(log());
    reset_log();
    // mem::replace, old used then dropped
    let old = mem::replace(&mut o.s, mk(10));
    print_u32(old.n + o.s.n);
    print_u32(drops());
    print_bool(a(&o.s) == at);
    drop(old);
    print_u32(drops());
    print_u64(log());
    reset_log();
    // the heap field alone reassigned, then the whole struct again
    o.s.v = vec![Tag(20), Tag(21), Tag(22)];
    print_u64(log());
    reset_log();
    o.s.b = Box::new(Tag(23));
    print_u64(log());
    reset_log();
    o.s = mk(30);
    print_u64(log());
    print_u32(drops());
    reset_log();
    // swap two whole structs, then drop one
    let mut other = mk(40);
    mem::swap(&mut o.s, &mut other);
    print_u32(o.s.n * 100 + other.n);
    print_bool(a(&o.s) == at);
    drop(other);
    print_u64(log());
    reset_log();
    // Option<S> field: take/replace/insert
    let mut opt: Option<S> = Some(mk(50));
    let taken = opt.take().unwrap();
    opt = Some(mk(60));
    print_u32(taken.n + opt.as_ref().unwrap().n);
    drop(taken);
    print_u64(log());
    reset_log();
    opt = None;
    print_u64(log());
    reset_log();
    print_u32(o.k + o.s.n);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    e_whole();
    print_u32(drops());
    print_u64(log());
}
