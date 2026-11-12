// heap_cycle's shape with Strings: allocate, ask the address, drop -- a small String and a large
// one -- next to a Box and a grown Vec.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn once_string(i: u32) -> usize {
    let mut s: String = black_box(String::with_capacity(100));
    s.push('a');
    if i % 2 == 0 { s.push('b'); }
    let a = s.as_ptr() as usize;
    drop(s);
    a
}

#[inline(never)]
fn once_big_string(i: u32) -> usize {
    let mut s: String = black_box(String::with_capacity(5_000));
    s.push('a');
    if i % 2 == 0 { s.push('b'); }
    let a = s.as_ptr() as usize;
    drop(s);
    a
}

#[inline(never)]
fn once_box(i: u32) -> usize {
    let b: Box<[u32; 2000]> = black_box(Box::new([i; 2000]));
    let a = &*b as *const [u32; 2000] as usize;
    drop(b);
    a
}

#[inline(never)]
fn once_grown(i: u32) -> usize {
    let mut v: Vec<u64> = Vec::new();
    let mut n = 0u64;
    while n < 3000 {
        v.push(n + i as u64);
        n += 1;
    }
    let a = black_box(&v).as_ptr() as usize;
    a
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut ok = true;
    let mut i = 0u32;
    while i < 30 {
        if once_grown(i) == 0 { ok = false; }
        if once_box(i) == 0 { ok = false; }
        i += 1;
    }
    print_bool(ok);
    let mut ok = true;
    let mut i = 0u32;
    while i < 200 {
        if once_string(i) == 0 { ok = false; }
        i += 1;
    }
    print_bool(ok);
    let mut ok = true;
    let mut i = 0u32;
    while i < 3_100_000 {
        if once_big_string(i) == 0 { ok = false; }
        i += 1;
    }
    print_bool(ok);
}
