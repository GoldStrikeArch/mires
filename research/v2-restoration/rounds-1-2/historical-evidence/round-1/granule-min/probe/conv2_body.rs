// The address of a Vec's buffer is observed where the Vec is made; the Vec is then moved by value
// into code that uses it only as a Vec and keeps it alive. Its address is never asked again.
// While it lives, natively no other live allocation can be at that address.
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn make(out: &mut usize, seed: u8) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.push(seed);
    v.push(2);
    *out = v.as_ptr() as usize;
    v
}

#[inline(never)]
fn plain(seed: u8) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.push(seed);
    v
}

#[inline(never)]
fn grow(mut v: Vec<u8>) -> Vec<u8> {
    v.push(3);
    v.push(4);
    v
}

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
    for x in v.iter() {
        s += *x as u32;
    }
    s
}

#[inline(never)]
fn consume(v: Vec<u8>) -> u32 {
    let mut s = 0u32;
    for x in v.iter() {
        s += *x as u32;
    }
    s + v.len() as u32
}

// A new block of the same size whose address is asked; the block is returned so it stays alive.
#[inline(never)]
fn fresh(out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.push(9);
    *out = v.as_ptr() as usize;
    v
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // other call sites that never look at an address, so `grow`, `hold`, `consume` see plain Vecs
    let g0 = grow(plain(1));
    let h0 = hold(plain(2), 1);
    print_u32(consume(plain(3)) + total(&g0) + total(&h0.items) + h0.tag);

    // E1: through `grow`
    let mut p = 0usize;
    let v = make(&mut p, 10);
    let w = grow(v);
    let mut a1 = 0usize;
    let mut a2 = 0usize;
    let k1 = fresh(&mut a1);
    let k2 = fresh(&mut a2);
    print_bool(a1 == p || a2 == p);
    print_bool(a1 == a2);
    print_u32(total(&w) + w.len() as u32);

    // E2: into a struct that stays alive
    let mut p2 = 0usize;
    let v2 = make(&mut p2, 20);
    let h = hold(v2, 5);
    let mut b1 = 0usize;
    let mut b2 = 0usize;
    let m1 = fresh(&mut b1);
    let m2 = fresh(&mut b2);
    print_bool(b1 == p2 || b2 == p2);
    print_u32(total(&h.items) + h.tag);

    // E3: into a Vec of Vecs
    let mut outer: Vec<Vec<u8>> = Vec::new();
    let mut p3 = 0usize;
    let v3 = make(&mut p3, 30);
    outer.push(v3);
    outer.push(plain(4));
    let mut c1 = 0usize;
    let mut c2 = 0usize;
    let n1 = fresh(&mut c1);
    let n2 = fresh(&mut c2);
    print_bool(c1 == p3 || c2 == p3);
    print_u32(total(&outer[0]) + outer.len() as u32);

    // E4: consumed (dropped inside): afterwards the address MAY be reused, nothing is printed about it
    let mut p4 = 0usize;
    let v4 = make(&mut p4, 40);
    print_u32(consume(v4));

    // E5: a loop of E1 with the moved Vecs all kept alive
    let mut alive: Vec<Vec<u8>> = Vec::new();
    let mut seen: Vec<usize> = Vec::new();
    let mut clash = false;
    let mut i = 0u8;
    while i < 50 {
        let mut pi = 0usize;
        let vi = make(&mut pi, i);
        let wi = grow(vi);
        let mut j = 0;
        while j < seen.len() {
            if seen[j] == pi { clash = true; }
            j += 1;
        }
        seen.push(pi);
        alive.push(wi);
        i += 1;
    }
    print_bool(clash);
    let mut sum = 0u32;
    let mut j = 0;
    while j < alive.len() {
        sum += total(&alive[j]);
        j += 1;
    }
    print_u32(sum + total(&k1) + total(&k2) + total(&m1) + total(&m2) + total(&n1) + total(&n2));
    black_box(&seen);
}
