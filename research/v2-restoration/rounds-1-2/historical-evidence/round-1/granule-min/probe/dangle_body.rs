// The same raw pointer turned into an integer twice: once while its Vec is alive, once after the
// Vec was dropped and another block allocated. A pointer's integer value does not depend on
// whether what it points to is still alive, so the two are equal -- always.
// (The program shape is heap_cycle's, where `drop(Vec<u8>)` goes through the native carrier.)
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn once_vec(i: u32) -> bool {
    let mut v: Vec<u8> = black_box(Vec::with_capacity(100));
    v.push(i as u8);
    let p = v.as_ptr();
    let a0 = p as usize;
    drop(v);
    let w: Vec<u8> = black_box(Vec::with_capacity(100));
    let w0 = w.as_ptr() as usize;
    let a1 = p as usize;
    black_box(w0);
    drop(w);
    a0 == a1
}

#[inline(never)]
fn once_big(i: u32) -> bool {
    let mut v: Vec<u8> = black_box(Vec::with_capacity(100_000));
    v.push(i as u8);
    let p = v.as_ptr();
    let a0 = p as usize;
    drop(v);
    let w: Vec<u8> = black_box(Vec::with_capacity(100_000));
    let w0 = w.as_ptr() as usize;
    let a1 = p as usize;
    black_box(w0);
    drop(w);
    a0 == a1
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
    let mut same = 0u32;
    let mut i = 0u32;
    while i < 1000 {
        if once_vec(i) { same += 1; }
        i += 1;
    }
    print_u32(same);
    let mut same = 0u32;
    let mut i = 0u32;
    while i < 1000 {
        if once_big(i) { same += 1; }
        i += 1;
    }
    print_u32(same);
}
