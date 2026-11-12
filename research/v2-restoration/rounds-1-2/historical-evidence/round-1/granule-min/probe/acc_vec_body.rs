// Allocate, ask the address, free -- in a loop. A native allocator reuses the block; whatever
// the synthetic address space does, the loop must run to its end.
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn once_vec(i: u32) -> usize {
    let mut v: Vec<u8> = black_box(Vec::with_capacity(5_000));
    v.push(i as u8);
    let a = v.as_ptr() as usize;
    drop(v);
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
    while i < 300 {
        if once_grown(i) == 0 { ok = false; }
        i += 1;
    }
    print_bool(ok);
    let mut ok = true;
    let mut i = 0u32;
    while i < 2_000 {
        if once_box(i) == 0 { ok = false; }
        i += 1;
    }
    print_bool(ok);
    let mut ok = true;
    let mut i = 0u32;
    while i < 3_100_000 {
        if once_vec(i) == 0 { ok = false; }
        i += 1;
    }
    print_bool(ok);
}
