//! (m) aggregate-valued collections: every element move goes through the write-through store
//! (`_put`, `ptr::copy`, swaps). Values only; a checksum per operation.
#![no_std]
#![no_main]
extern crate alloc;
#[path = "prelude.rs"]
mod prelude;
use alloc::collections::{BinaryHeap, VecDeque};
use alloc::vec::Vec;
use core::hint::black_box;
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

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct K {
    a: u32,
    b: u32,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Rec<'a> {
    key: K,
    inner: (u32, K),
    tag: &'a K,
    opt: Option<K>,
}
fn rec<'a>(i: u32, tags: &'a [K; 3]) -> Rec<'a> {
    let key = K { a: (i * 7919) % 101, b: i };
    Rec { key, inner: (i * 3, K { a: i, b: i + 1 }), tag: &tags[(i % 3) as usize], opt: if i % 4 == 0 { None } else { Some(K { a: i, b: 0 }) } }
}
fn hash(acc: u32, r: &Rec) -> u32 {
    let mut h = acc.wrapping_mul(31).wrapping_add(r.key.a);
    h = h.wrapping_mul(31).wrapping_add(r.key.b);
    h = h.wrapping_mul(31).wrapping_add(r.inner.0);
    h = h.wrapping_mul(31).wrapping_add(r.inner.1.a + r.inner.1.b);
    h = h.wrapping_mul(31).wrapping_add(r.tag.a);
    h.wrapping_mul(31).wrapping_add(r.opt.map_or(77, |k| k.a))
}
fn sum(v: &[Rec]) -> u32 {
    let mut h = 17;
    for r in v {
        h = hash(h, r);
    }
    h
}
fn tags_ok(t: &[K; 3]) -> bool {
    t[0].a == 1000 && t[1].a == 2000 && t[2].a == 3000 && t[0].b == 1 && t[1].b == 2 && t[2].b == 3
}

#[inline(never)]
fn t_vec(tags: &[K; 3]) {
    print_str("vec");
    let mut v: Vec<Rec> = Vec::new();
    for i in 0..40 {
        v.push(rec(i, tags));
    }
    print_u32(sum(&v));
    v.sort();
    print_u32(sum(&v));
    v.sort_unstable_by(|x, y| y.key.b.cmp(&x.key.b));
    print_u32(sum(&v));
    v.sort_by_key(|r| (r.inner.0 % 7, r.key.a));
    print_u32(sum(&v));
    v.reverse();
    rot_left(&mut v, 13);
    rot_right(&mut v, 5);
    print_u32(sum(&v));
    let r = v.remove(3);
    v.insert(20, r);
    let r = v.swap_remove(1);
    v.push(r);
    v.swap(0, 39);
    print_u32(sum(&v));
    v.retain(|r| r.key.b % 5 != 0);
    print_u32(sum(&v));
    v.dedup_by_key(|r| r.inner.0 % 2);
    print_u32(sum(&v));
    let tail: Vec<Rec> = v.drain(1..).collect();
    print_u32(sum(&v) ^ sum(&tail));
    v.extend(tail.iter().cloned());
    v.extend(tail);
    v.truncate(9);
    let w = v.clone();
    v[0] = v[3].clone();
    v[3].inner.1.a += 5;
    v.copy_from_slice_like(&w);
    print_u32(sum(&v) ^ sum(&w));
    let mut arr = [rec(1, tags), rec(2, tags), rec(3, tags), rec(4, tags), rec(5, tags)];
    arr.sort_by(|x, y| y.cmp(x));
    rot_left(&mut arr, 2);
    arr.swap(0, 4);
    arr.reverse();
    print_u32(sum(&arr));
    print_bool(tags_ok(tags));
}

trait CopyLike<T> {
    fn copy_from_slice_like(&mut self, src: &[T]);
}
impl<T: Clone> CopyLike<T> for Vec<T> {
    fn copy_from_slice_like(&mut self, src: &[T]) {
        self.clone_from_slice(src);
    }
}

#[inline(never)]
fn t_deque_heap(tags: &[K; 3]) {
    print_str("deque_heap");
    let mut d: VecDeque<Rec> = VecDeque::new();
    for i in 0..25 {
        if i % 3 == 0 {
            d.push_front(rec(i, tags));
        } else {
            d.push_back(rec(i, tags));
        }
        if i % 5 == 4 {
            let f = d.pop_front().unwrap();
            d.push_back(f);
        }
    }
    for _ in 0..7 { let f = d.pop_front().unwrap(); d.push_back(f); }
    d.swap(0, 5);
    let mut h = 17;
    for r in d.iter() {
        h = hash(h, r);
    }
    print_u32(h);
    let x = d.remove(4).unwrap();
    d.insert(10, x);
    let mut tmp: Vec<Rec> = d.drain(..).collect();
    tmp.sort();
    d.extend(tmp);
    let mut h = 17;
    while let Some(r) = d.pop_back() {
        h = hash(h, &r);
    }
    print_u32(h);
    let mut heap: BinaryHeap<Rec> = BinaryHeap::new();
    for i in 0..30 {
        heap.push(rec(i, tags));
        if i % 4 == 3 {
            let top = heap.pop().unwrap();
            h = hash(h, &top);
        }
    }
    while let Some(r) = heap.pop() {
        h = hash(h, &r);
    }
    print_u32(h);
    print_bool(tags_ok(tags));
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let tags = black_box([K { a: 1000, b: 1 }, K { a: 2000, b: 2 }, K { a: 3000, b: 3 }]);
    t_vec(&tags);
    t_deque_heap(&tags);
}
