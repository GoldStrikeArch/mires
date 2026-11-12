//! Reading a live heap block as text never gives its synthetic address away.
//!
//! A `&str` made over bytes a live `Vec<u8>` or `String` still owns — `str::from_utf8_unchecked`
//! over the vector, a `*const str` built from the buffer pointer and a length, `String::as_str` —
//! is decoded by `__rt.bytes_str`, which only reads the block. Only a conversion that ends the
//! value's life (a drop through a native carrier, `__rt.drop_block_str` and `__rt.drop_block_nv`)
//! gives a block's address granule back. Each owner's address is observed first; the text is read
//! through every one of those forms, repeatedly; then more blocks of the same size are allocated
//! and addressed while the owners are alive. Natively none of them can share an owner's address,
//! and the owners still hold their bytes, which is all that is printed.
//!
//! The expectations were validated by transplanting this file onto `std` and running it at `-O0`
//! and `-O3`, which agree.

#![feature(ptr_metadata)]


extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;

#[inline(never)]
fn bytes(out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(64);
    v.extend_from_slice(b"held bytes");
    *out = v.as_ptr() as usize;
    v
}

#[inline(never)]
fn text(out: &mut usize) -> String {
    let mut s = String::with_capacity(64);
    s.push_str("held text");
    *out = s.as_bytes().as_ptr() as usize;
    s
}

#[inline(never)]
fn unchecked(v: &Vec<u8>) -> &str {
    // SAFETY: the bytes are ASCII.
    unsafe { core::str::from_utf8_unchecked(v) }
}

#[inline(never)]
fn from_parts(v: &Vec<u8>) -> &str {
    let p: *const str = core::ptr::from_raw_parts(v.as_ptr(), v.len());
    // SAFETY: `p` covers the vector's initialized ASCII bytes, which outlive the reference.
    unsafe { &*p }
}

#[inline(never)]
fn fresh(out: &mut usize) -> Vec<u8> {
    let mut v: Vec<u8> = black_box(Vec::with_capacity(64));
    v.push(1);
    *out = v.as_ptr() as usize;
    v
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut a = 0usize;
    let v = bytes(&mut a);
    let mut b = 0usize;
    let s = text(&mut b);
    print_bool(a != b);
    let mut n = 0usize;
    let mut i = 0u32;
    while i < 20 {
        n += unchecked(&v).len() + from_parts(&v).len() + s.as_str().len();
        i += 1;
    }
    print_usize(n);
    print_str(unchecked(&v));
    print_str(from_parts(&v));
    print_str(s.as_str());
    let mut clash = false;
    let mut live: Vec<Vec<u8>> = Vec::new();
    let mut i = 0u32;
    while i < 40 {
        let mut c = 0usize;
        let w = fresh(&mut c);
        if c == a || c == b {
            clash = true;
        }
        live.push(w);
        i += 1;
    }
    print_bool(clash);
    print_bool(v.as_ptr() as usize == a);
    print_u32(live.len() as u32 + v.len() as u32 + s.len() as u32);
}
#[allow(dead_code)] fn print_bool(x: bool) { println!("{}", x) }
#[allow(dead_code)] fn print_u32(x: u32) { println!("{}", x) }
#[allow(dead_code)] fn print_usize(x: usize) { println!("{}", x) }
#[allow(dead_code)] fn print_str(x: &str) { println!("{}", x) }
fn main() { rust_entry() }
