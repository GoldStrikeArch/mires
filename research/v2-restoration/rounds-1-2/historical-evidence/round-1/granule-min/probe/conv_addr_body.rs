// A Vec whose buffer address was observed, then moved by value through functions that only use it
// as a Vec. Moving a Vec never moves its heap buffer, so natively: the buffer's address is the same
// before and after the move (no reallocation happens here: the capacity is never exceeded), a raw
// pointer into it keeps its integer value, and no other live allocation has that address.
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr;

#[inline(never)]
fn make(out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.push(1);
    v.push(2);
    *out = v.as_ptr() as usize;
    v
}

#[inline(never)]
fn make_ptr(out: &mut *const u8) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.push(5);
    v.push(6);
    *out = v.as_ptr();
    v
}

#[inline(never)]
fn make_big(out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(100_000);
    v.push(1);
    *out = v.as_ptr() as usize;
    v
}

// Uses the Vec only as a Vec: push within capacity, hand it back.
#[inline(never)]
fn grow(mut v: Vec<u8>) -> Vec<u8> {
    v.push(3);
    v
}

// Keeps the Vec alive inside a struct and hands that back.
pub struct Holder {
    pub items: Vec<u8>,
    pub tag: u32,
}

#[inline(never)]
fn hold(v: Vec<u8>, tag: u32) -> Holder {
    Holder { items: v, tag }
}

#[inline(never)]
fn total(v: &Vec<u8>) -> u32 {
    let mut s = 0u32;
    let mut i = 0;
    while i < v.len() {
        s += v[i] as u32;
        i += 1;
    }
    s
}

#[inline(never)]
fn fresh(cap: usize) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(cap);
    v.push(9);
    v
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // A: the integer address of the buffer, taken before the move.
    let mut p = 0usize;
    let v = make(&mut p);
    let w = grow(v);
    // other blocks of the same size, allocated while `w` is alive: none is at `p`
    let r1 = fresh(64);
    let r2 = fresh(64);
    let r3 = black_box(Box::new([0u8; 64]));
    print_bool(r1.as_ptr() as usize == p);
    print_bool(r2.as_ptr() as usize == p);
    print_bool((&*r3 as *const [u8; 64] as usize) == p);
    print_u32(total(&w));
    let q = w.as_ptr() as usize;
    print_bool(q == p);
    print_bool(q == r1.as_ptr() as usize || q == r2.as_ptr() as usize);
    print_bool(r1.as_ptr() as usize == r2.as_ptr() as usize);

    // B: a raw pointer into the buffer, kept across the move.
    let mut rp: *const u8 = ptr::null();
    let v2 = make_ptr(&mut rp);
    let a1 = rp as usize;
    let w2 = grow(v2);
    let a2 = rp as usize;
    print_bool(a1 == a2);
    let s1 = fresh(64);
    let s2 = fresh(64);
    print_bool(s1.as_ptr() as usize == a1 || s2.as_ptr() as usize == a1);
    print_bool(s1.as_ptr() as usize == a2 || s2.as_ptr() as usize == a2);
    print_bool(rp as usize == a1);
    print_bool(ptr::eq(rp, w2.as_ptr()));
    print_bool(w2.as_ptr() as usize == a1);
    print_u32(total(&w2) + unsafe { *rp } as u32);

    // C: the Vec moved into a struct that stays alive; a large buffer this time.
    let mut pb = 0usize;
    let vb = make_big(&mut pb);
    let h = hold(vb, 7);
    let t1 = fresh(100_000);
    let t2 = fresh(100_000);
    print_bool(t1.as_ptr() as usize == pb || t2.as_ptr() as usize == pb);
    let t1a = t1.as_ptr() as usize;
    print_bool(pb >= t1a && pb < t1a + 100_000);
    print_bool(h.items.as_ptr() as usize == pb);
    print_u32(h.tag + h.items.len() as u32 + t1.len() as u32 + t2.len() as u32);

    // D: the same moves in a loop, the address taken again every round: it never changes, and
    // the block allocated in between is never there.
    let mut p0 = 0usize;
    let mut cur = make(&mut p0);
    let mut stable = true;
    let mut clash = false;
    let mut i = 0u32;
    while i < 200 {
        cur = grow(cur);
        if cur.len() > 60 { cur.clear(); }
        let other = fresh(64);
        if other.as_ptr() as usize == p0 { clash = true; }
        if cur.as_ptr() as usize != p0 { stable = false; }
        i += 1;
    }
    print_bool(stable);
    print_bool(clash);
    print_u32(total(&r1) + total(&r2) + total(&s1) + total(&s2) + r3[0] as u32 + cur.len() as u32);
}
