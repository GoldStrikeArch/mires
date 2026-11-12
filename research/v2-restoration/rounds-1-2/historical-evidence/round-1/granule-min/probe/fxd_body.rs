use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;

/// A raw pointer's integer value, taken while its `Vec` is alive and again after the `Vec` was
/// dropped and another block of the same size allocated and addressed: always the same.
#[inline(never)]
fn vec_dangles(i: u32) -> bool {
    let mut v: Vec<u8> = black_box(Vec::with_capacity(100));
    v.push(i as u8);
    let p = v.as_ptr();
    let before = p as usize;
    drop(v);
    let w: Vec<u8> = black_box(Vec::with_capacity(100));
    let other = w.as_ptr() as usize;
    let after = p as usize;
    black_box(other);
    drop(w);
    before == after
}

/// The same for a `String` that is written to.
#[inline(never)]
fn string_dangles(i: u32) -> bool {
    let mut s: String = black_box(String::with_capacity(100));
    s.push('a');
    if i % 2 == 0 {
        s.push('b');
    }
    let p = s.as_ptr();
    let before = p as usize;
    drop(s);
    let t: String = black_box(String::with_capacity(100));
    let other = t.as_ptr() as usize;
    let after = p as usize;
    black_box(other);
    drop(t);
    before == after
}

/// A struct whose fields are named after the runtime's own record keys.
pub struct Rec {
    pub buf: u32,
    pub off: u32,
    pub len: u32,
    pub cap: u32,
    pub text: u32,
}

#[inline(never)]
fn records(i: u32) -> (usize, u32) {
    let mut v: Vec<Rec> = black_box(Vec::with_capacity(4));
    let mut k = 0;
    while k < 3 {
        v.push(Rec { buf: i, off: k, len: 1000, cap: 2000, text: 7 });
        k += 1;
    }
    let a = v.as_ptr() as usize;
    let mut sum = 0u32;
    for r in v.iter() {
        sum = sum.wrapping_add(r.buf + r.off + r.len + r.cap + r.text);
    }
    drop(v);
    (a, sum)
}

#[inline(never)]
fn boxed(i: u32) -> usize {
    let b: Box<[u32; 200]> = black_box(Box::new([i; 200]));
    let a = &*b as *const [u32; 200] as usize;
    drop(b);
    a
}

#[inline(never)]
fn grown(i: u32) -> usize {
    let mut v: Vec<u64> = Vec::new();
    let mut n = 0u64;
    while n < 300 {
        v.push(n + i as u64);
        n += 1;
    }
    black_box(&v).as_ptr() as usize
}

#[inline(never)]
fn fresh(out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = black_box(Vec::with_capacity(64));
    v.push(1);
    *out = v.as_ptr() as usize;
    v
}

#[inline(never)]
fn fresh_string(out: &mut usize) -> String {
    let mut s: String = black_box(String::with_capacity(64));
    s.push('z');
    *out = s.as_ptr() as usize;
    s
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut nonnull = true;
    let mut i = 0u32;
    while i < 30 {
        if grown(i) == 0 || boxed(i) == 0 {
            nonnull = false;
        }
        i += 1;
    }
    print_bool(nonnull);

    // 1. Dangling pointers keep their integer value across the release.
    let mut same = 0u32;
    let mut i = 0u32;
    while i < 500 {
        if vec_dangles(i) {
            same += 1;
        }
        if string_dangles(i) {
            same += 1;
        }
        i += 1;
    }
    print_u32(same);

    // 2. A live block never shares its address with another live block, while blocks around it
    //    are made, addressed and consumed.
    let mut live: Vec<Vec<u8>> = Vec::new();
    let mut live_s: Vec<String> = Vec::new();
    let mut seen: Vec<usize> = Vec::new();
    let mut duplicate = false;
    let mut i = 0u32;
    while i < 200 {
        if !vec_dangles(i) || !string_dangles(i) {
            duplicate = true;
        }
        let mut a = 0usize;
        let v = fresh(&mut a);
        let mut b = 0usize;
        let s = fresh_string(&mut b);
        let mut j = 0;
        while j < seen.len() {
            if seen[j] == a || seen[j] == b {
                if !duplicate { print_u32(i); print_u32(j as u32); print_usize(seen[j]); print_usize(a); print_usize(b); }
                duplicate = true;
            }
            j += 1;
        }
        if a == b {
            if !duplicate { print_u32(9999); print_u32(i); }
            duplicate = true;
        }
        seen.push(a);
        seen.push(b);
        live.push(v);
        live_s.push(s);
        i += 1;
    }
    print_bool(duplicate);

    // 3. A live String whose bytes are only read keeps its address to itself.
    let mut held = String::with_capacity(64);
    held.push_str("held");
    let address = held.as_ptr() as usize;
    print_str(held.as_str());
    let mut clash = false;
    let mut keep: Vec<String> = Vec::new();
    let mut i = 0u32;
    while i < 50 {
        let mut b = 0usize;
        let s = fresh_string(&mut b);
        if b == address {
            clash = true;
        }
        keep.push(s);
        i += 1;
    }
    print_bool(clash);
    print_str(held.as_str());

    // 4. Records whose field names are the runtime's: their contents survive, their Vec's address
    //    is a real one.
    let mut total = 0u32;
    let mut nonnull = true;
    let mut i = 0u32;
    while i < 100 {
        let (a, sum) = records(i);
        if a == 0 {
            nonnull = false;
        }
        total = total.wrapping_add(sum);
        i += 1;
    }
    print_bool(nonnull);
    print_u32(total);
    print_u32(live.len() as u32 + live_s.len() as u32 + keep.len() as u32);
}
