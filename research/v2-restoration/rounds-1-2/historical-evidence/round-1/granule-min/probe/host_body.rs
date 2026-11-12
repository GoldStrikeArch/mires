// Shapes meant to reach the baseline -> host-string conversion: an immutable String whose address
// is observed, then moved into a function that only reads it, or dropped.
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn measure(s: String) -> usize {
    s.len()
}

#[inline(never)]
fn count_a(s: String) -> u32 {
    let mut n = 0;
    for c in s.chars() {
        if c == 'a' {
            n += 1;
        }
    }
    n
}

#[inline(never)]
fn plain(i: u32) -> String {
    if i % 2 == 0 {
        String::from("banana")
    } else {
        String::from("apple")
    }
}

#[inline(never)]
fn once_host(i: u32) -> usize {
    let s: String = black_box(plain(i));
    let a = s.as_ptr() as usize;
    black_box(measure(s));
    a
}

#[inline(never)]
fn once_count(i: u32) -> usize {
    let s: String = black_box(plain(i));
    let a = s.as_ptr() as usize;
    black_box(count_a(s));
    a
}

#[inline(never)]
fn once_drop(i: u32) -> usize {
    let s: String = black_box(plain(i));
    let a = s.as_ptr() as usize;
    drop(s);
    a
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut total = 0usize;
    let mut i = 0u32;
    while i < 10 {
        total += measure(plain(i));
        total += count_a(plain(i)) as usize;
        i += 1;
    }
    print_usize(total);
    let mut ok = true;
    let mut i = 0u32;
    while i < ROUNDS {
        if once_host(i) == 0 || once_count(i) == 0 || once_drop(i) == 0 {
            ok = false;
        }
        i += 1;
    }
    print_bool(ok);
    let v: Vec<u8> = Vec::new();
    black_box(v);
}
