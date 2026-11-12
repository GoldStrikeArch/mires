// A String whose address is observed where it is made, then handed by value (a fresh call result
// used once) to functions that only read it and drop it, ROUNDS times.
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn make_s(out: &mut usize, i: u32) -> String {
    let mut s = String::with_capacity(5000);
    s.push(if i % 2 == 0 { 'e' } else { 'o' });
    *out = s.as_ptr() as usize;
    s
}

#[inline(never)]
fn measure(s: String) -> usize {
    s.len()
}

#[inline(never)]
fn first_is_e(s: String) -> bool {
    s.as_str().starts_with('e')
}

#[inline(never)]
fn into_text(s: String) -> u32 {
    let mut n = 0;
    for c in s.chars() {
        n += c as u32;
    }
    n
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut total = 0usize;
    let mut i = 0u32;
    while i < 20 {
        total += measure(String::from("banana"));
        if first_is_e(String::from("eagle")) {
            total += 1;
        }
        total += into_text(String::from("x")) as usize;
        i += 1;
    }
    print_usize(total);
    let mut ok = true;
    let mut i = 0u32;
    while i < ROUNDS {
        let mut p = 0usize;
        let n = measure(make_s(&mut p, i)) + into_text(make_s(&mut p, i)) as usize;
        if p == 0 || n == 0 {
            ok = false;
        }
        if first_is_e(make_s(&mut p, i)) != (i % 2 == 0) {
            ok = false;
        }
        i += 1;
    }
    print_bool(ok);
    let v: Vec<u8> = Vec::new();
    black_box(v);
}
