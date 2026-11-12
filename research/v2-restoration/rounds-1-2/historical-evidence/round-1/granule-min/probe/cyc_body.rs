// Allocate, ask the address, drop -- ROUNDS times, for a Vec<u8>, a String that is only read
// (host string carrier) and a String that is pushed to (builder carrier). Every drop crosses into a
// native carrier. Natively the allocator reuses the block every round; the loop must finish.
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn once_vec(i: u32) -> usize {
    let mut v: Vec<u8> = black_box(Vec::with_capacity(100));
    v.push(i as u8);
    let a = v.as_ptr() as usize;
    drop(v);
    a
}

#[inline(never)]
fn once_string(i: u32) -> usize {
    let mut s: String = black_box(String::with_capacity(100));
    s.push('a');
    if i % 2 == 0 {
        s.push('b');
    }
    let a = s.as_ptr() as usize;
    drop(s);
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
        if once_grown(i) == 0 {
            ok = false;
        }
        i += 1;
    }
    print_bool(ok);
    let mut ok = true;
    let mut i = 0u32;
    while i < ROUNDS {
        if once_vec(i) == 0 {
            ok = false;
        }
        i += 1;
    }
    print_bool(ok);
    let mut ok = true;
    let mut i = 0u32;
    while i < ROUNDS {
        if once_string(i) == 0 {
            ok = false;
        }
        i += 1;
    }
    print_bool(ok);
}
