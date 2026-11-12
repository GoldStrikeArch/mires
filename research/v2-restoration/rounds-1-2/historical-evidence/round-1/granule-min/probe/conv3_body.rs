// A Vec / String whose buffer address is observed where it is made, then moved by value into
// code that only uses it as a Vec / String and keeps it alive. While it lives, natively no other
// live allocation can be at the observed address. Only Rust-guaranteed relations are printed.
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn make(out: &mut usize, seed: u8) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.push(seed);
    *out = v.as_ptr() as usize;
    v
}

#[inline(never)]
fn make_s(out: &mut usize) -> String {
    let mut s = String::with_capacity(64);
    s.push('q');
    *out = s.as_ptr() as usize;
    s
}

#[inline(never)]
fn plain(seed: u8) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.push(seed);
    v
}

#[inline(never)]
fn plain_s(seed: u32) -> String {
    let mut s = String::with_capacity(64);
    if seed % 2 == 0 {
        s.push('e');
    } else {
        s.push('o');
    }
    s
}

#[inline(never)]
fn keep(mut v: Vec<u8>, extra: u8) -> Vec<u8> {
    v.push(extra);
    v.push(extra);
    v
}

#[inline(never)]
fn keep_s(mut s: String) -> String {
    s.push('x');
    s.push_str("yz");
    s
}

#[inline(never)]
fn fresh(out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.push(9);
    *out = v.as_ptr() as usize;
    v
}

#[inline(never)]
fn fresh_s(out: &mut usize) -> String {
    let mut s = String::with_capacity(64);
    s.push('f');
    *out = s.as_ptr() as usize;
    s
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Many plain users, so `keep` and `keep_s` look like native-carrier functions.
    let mut total = 0u32;
    let mut i = 0u32;
    while i < 20 {
        let w = keep(plain(i as u8), 1);
        total += w.len() as u32;
        let t = keep_s(plain_s(i));
        total += t.len() as u32;
        i += 1;
    }
    print_u32(total);

    let mut p = 0usize;
    let v = make(&mut p, 3);
    let kept = keep(v, 4);
    let mut q = 0usize;
    let s = make_s(&mut q);
    let kept_s = keep_s(s);
    let mut clash = false;
    let mut alive: Vec<Vec<u8>> = Vec::new();
    let mut alive_s: Vec<String> = Vec::new();
    let mut j = 0u32;
    while j < 50 {
        let mut a = 0usize;
        let f = fresh(&mut a);
        if a == p || a == q {
            clash = true;
        }
        alive.push(f);
        let mut b = 0usize;
        let g = fresh_s(&mut b);
        if b == p || b == q {
            clash = true;
        }
        alive_s.push(g);
        j += 1;
    }
    print_bool(clash);
    print_u32(kept.len() as u32 + kept_s.len() as u32 + alive.len() as u32 + alive_s.len() as u32);
    black_box(&kept);
    black_box(&kept_s);
}
