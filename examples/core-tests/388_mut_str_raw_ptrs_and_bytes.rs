//! Low-level `&mut str` / `*mut str` byte mutations, raw pointers, casts, and containment (MIRES-025).
//!
//! Covers:
//! - `str::as_bytes_mut` replacing multibyte UTF-8 `"é"` (2 bytes) with `"ab"` (2 bytes) and back
//! - `str::as_mut_ptr` direct byte writes into `String` and `[u8; N]`
//! - `*mut str` equality (`slice_ptr_eq`) on equal-text disjoint halves vs identical slices
//! - `as_mut_ptr` pointer identity, `core::ptr::metadata`, `core::mem::size_of_val`
//! - `core::ptr::from_raw_parts_mut::<str>` (live buffer & empty dangling pointer) and `from_raw_parts`
//! - `*mut str -> *const str` (`MutToConstPointer`), `&mut str <-> &mut [u8]` transmute
//! - `#[repr(transparent)]` wrappers around `str` and `&mut str`
//! - Containment: casting `&str` -> `*mut str` -> `*mut [u8]` and mutating never corrupts `&str`
//! - `static mut` byte array mutated through `from_utf8_unchecked_mut`

#![feature(ptr_metadata)]
#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use core::hint::black_box;
use core::mem;
use core::ptr::{self, NonNull};

#[repr(transparent)]
struct TransparentStr(str);

#[repr(transparent)]
struct WrapMut<'a>(&'a mut str);

static mut STATIC_BUF: [u8; 8] = *b"StAtIc_0";

#[inline(never)]
fn raw_mut_to_const(p: *mut str) -> *const str {
    p
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. as_bytes_mut replacing multibyte UTF-8 "é" (2 bytes) <-> "ab" (2 bytes) in place
    let mut s = String::from(black_box("xéy"));
    unsafe {
        let bytes = s.as_bytes_mut();
        print_usize(bytes.len());
        bytes[1] = b'a';
        bytes[2] = b'b';
    }
    print_str(s.as_str());
    unsafe {
        let bytes = s.as_bytes_mut();
        bytes[1] = 0xc3;
        bytes[2] = 0xa9;
    }
    print_str(s.as_str());

    // 2. as_mut_ptr direct writes into String
    let mut s2 = String::from(black_box("hello"));
    unsafe {
        let p = s2.as_mut_str().as_mut_ptr();
        *p = b'H';
        *p.add(4) = b'O';
    }
    print_str(s2.as_str());

    // 3. *mut str pointer equality (disjoint equal-text halves vs same slice) & metadata/size_of_val
    let mut dup = String::from(black_box("aaaa"));
    {
        let (a, b) = dup.split_at_mut(2);
        let pa: *mut str = a;
        let pb: *mut str = b;
        #[allow(ambiguous_wide_pointer_comparisons)]
        {
            print_bool(pa == pa);
            print_bool(pa == pb);
        }
        print_bool(ptr::eq(a.as_mut_ptr(), b.as_mut_ptr()));
        print_usize(ptr::metadata(pa));
        print_usize(mem::size_of_val(a));
        print_usize(mem::size_of_val(&*a));
        // Coerce *mut str -> *const str (MutToConstPointer)
        let ca: *const str = raw_mut_to_const(pa);
        print_usize(ptr::metadata(ca));
        print_str(unsafe { &*ca });
        a.make_ascii_uppercase();
        print_str(&*a);
        print_str(&*b);
    }
    print_str(dup.as_str());

    // 4. ptr::from_raw_parts_mut::<str> (live buffer & empty dangling) + ptr::from_raw_parts::<str>
    let mut raw_buf = *b"RaW_pArT";
    unsafe {
        let p_mut: *mut str = ptr::from_raw_parts_mut(raw_buf.as_mut_ptr(), raw_buf.len());
        (&mut *p_mut).make_ascii_lowercase();
        let p_const: *const str = ptr::from_raw_parts(raw_buf.as_ptr(), raw_buf.len());
        print_str(&*p_const);

        let empty_mut: *mut str = ptr::from_raw_parts_mut(NonNull::<u8>::dangling().as_ptr(), 0);
        print_usize(ptr::metadata(empty_mut));
        print_bool((&*empty_mut).is_empty());
        print_str(&*empty_mut);
    }

    // 5. transmute <&mut str, &mut [u8]> and back
    let mut tr = String::from(black_box("TrAnSmUtE"));
    unsafe {
        let bs: &mut [u8] = mem::transmute::<&mut str, &mut [u8]>(tr.as_mut_str());
        bs[0] = b't';
        bs[8] = b'e';
        let back: &mut str = mem::transmute::<&mut [u8], &mut str>(bs);
        back.make_ascii_lowercase();
    }
    print_str(tr.as_str());

    // 6. Transparent wrappers around str and &mut str
    let mut wrap_s = String::from(black_box("TrAnSp"));
    unsafe {
        let ts: &mut TransparentStr =
            &mut *(wrap_s.as_mut_str() as *mut str as *mut TransparentStr);
        ts.0.make_ascii_lowercase();
        print_str(&ts.0);
    }
    print_str(wrap_s.as_str());

    let mut wrap_m = String::from(black_box("wRaPmUt"));
    {
        let w = WrapMut(wrap_m.as_mut_str());
        w.0.make_ascii_uppercase();
        print_str(&*w.0);
    }
    print_str(wrap_m.as_str());

    // 7. Round-trip cast: immutable &str -> *const str -> *mut str -> *const str (read-only,
    // UB-free on native) and verifying metadata + content survive cleanly.
    let lit1: &str = black_box("KeepMe-é");
    let p_mut: *mut str = lit1 as *const str as *mut str;
    print_usize(ptr::metadata(p_mut));
    let p_const: *const str = raw_mut_to_const(p_mut);
    print_str(unsafe { &*p_const });
    print_str(lit1);

    // 8. static mut buffer mutated via from_utf8_unchecked_mut
    unsafe {
        let p = core::ptr::addr_of_mut!(STATIC_BUF) as *mut u8;
        let s_static = core::str::from_utf8_unchecked_mut(core::slice::from_raw_parts_mut(p, 8));
        s_static.make_ascii_lowercase();
        print_str(&*s_static);
    }
}
