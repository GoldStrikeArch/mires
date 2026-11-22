//! Safe `Box::<[T]>::default()`, `Box::<str>::default()`, `Box::<[Z]>::from`, `Vec::<T>::new()`,
//! and provenance-free empty `str` constants (`*const str`, `NonNull<str>`, `&str`, `Option`,
//! `static`) across unwind, abort, inline+`-Oz`, and `js-str=bytes`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr::{self, NonNull};

static mut DROPPED: u32 = 0;

struct Z;
impl Drop for Z {
    fn drop(&mut self) {
        unsafe { DROPPED += 1 }
    }
}

#[repr(align(4))]
struct Align4;

#[repr(align(8))]
#[derive(Clone, Copy)]
struct Align8;

#[repr(C)]
struct Pair(u32, u32);

const Z4_SLICE: *const [Align4] =
    ptr::slice_from_raw_parts(NonNull::<Align4>::dangling().as_ptr(), 0);

struct SyncStrPtr(*const str);
unsafe impl Sync for SyncStrPtr {}

const EMPTY_STR_RAW: *const str =
    ptr::slice_from_raw_parts(NonNull::<u8>::dangling().as_ptr(), 0) as *const str;
const EMPTY_STR_NN: NonNull<str> = unsafe { NonNull::new_unchecked(EMPTY_STR_RAW as *mut str) };
const EMPTY_STR_REF: &str = unsafe { &*EMPTY_STR_RAW };
const OPT_STR_SOME: Option<&str> = Some(EMPTY_STR_REF);
const OPT_STR_NONE: Option<&str> = None;
const OPT_NN_STR_SOME: Option<NonNull<str>> = Some(EMPTY_STR_NN);
const OPT_NN_STR_NONE: Option<NonNull<str>> = None;

static S_EMPTY_STR_RAW: SyncStrPtr = SyncStrPtr(EMPTY_STR_RAW);
static S_EMPTY_STR_REF: &str = EMPTY_STR_REF;
static S_OPT_STR: Option<&str> = OPT_STR_SOME;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Safe Vec<Z> and Box<[Z]> default + drop
    let vz: Vec<Z> = Vec::new();
    let sz: &[Z] = &vz;
    print_usize(sz.len());
    print_bool(ptr::eq(sz.as_ptr(), NonNull::<Z>::dangling().as_ptr()));
    drop(vz);

    let bz: Box<[Z]> = Box::default();
    print_usize(bz.len());
    print_bool(ptr::eq(bz.as_ptr(), NonNull::<Z>::dangling().as_ptr()));
    drop(bz);

    let ba8: Box<[Align8]> = Box::from(&[Align8; 3][..]);
    print_usize(ba8.len());
    print_bool(ba8.as_ptr().is_aligned());
    print_usize(ba8.iter().count());
    drop(ba8);

    // Safe Box<[T]>::default() for non-ZST T + drop + empty iteration + ptr::eq with Vec::new()
    let vu32: Vec<u32> = Vec::new();
    let bu32: Box<[u32]> = Box::default();
    print_usize(bu32.len());
    print_usize(bu32.as_ptr() as usize);
    print_bool(ptr::eq(bu32.as_ptr(), NonNull::<u32>::dangling().as_ptr()));
    print_bool(ptr::eq(
        bu32.as_ptr().cast::<u8>(),
        NonNull::<u32>::dangling().as_ptr().cast::<u8>(),
    ));
    print_bool(ptr::eq(
        black_box(Z4_SLICE).cast::<u8>(),
        bu32.as_ptr().cast::<u8>(),
    ));
    print_bool(ptr::eq(bu32.as_ptr(), vu32.as_ptr()));
    let mut sum = 0u32;
    for &x in &*bu32 {
        sum += x;
    }
    print_u32(sum);
    drop(bu32);
    drop(vu32);

    let bu8: Box<[u8]> = Box::default();
    print_usize(bu8.len());
    print_usize(bu8.as_ptr() as usize);
    drop(bu8);

    let bu64: Box<[u64]> = Box::default();
    print_usize(bu64.len());
    print_usize(bu64.as_ptr() as usize);
    drop(bu64);

    let bchunk: Box<[[u8; 4]]> = Box::default();
    print_usize(bchunk.len());
    print_usize(bchunk.as_ptr() as usize);
    drop(bchunk);

    let bpair: Box<[Pair]> = Box::default();
    print_usize(bpair.len());
    print_usize(bpair.as_ptr() as usize);
    drop(bpair);

    let bstrings: Box<[String]> = Box::default();
    print_usize(bstrings.len());
    print_bool(bstrings.as_ptr().is_aligned());
    drop(bstrings);

    let bboxed: Box<[Box<u32>]> = Box::default();
    print_usize(bboxed.len());
    print_bool(bboxed.as_ptr().is_aligned());
    drop(bboxed);

    // Safe Box<str>::default() + Option<Box<[u32]>> + Option<Box<str>>
    let bstr: Box<str> = Box::default();
    print_usize(bstr.len());
    print_bool(bstr.is_empty());
    print_str(&bstr);
    drop(bstr);

    let opt_bu32: Option<Box<[u32]>> = Some(Box::default());
    print_usize(black_box(opt_bu32).unwrap().len());

    let opt_bstr: Option<Box<str>> = Some(Box::default());
    print_usize(black_box(opt_bstr).unwrap().len());

    // Provenance-free empty str constants and statics
    let sr: &str = unsafe { &*black_box(EMPTY_STR_RAW) };
    print_usize(sr.len());
    print_bool(sr.is_empty());
    print_usize(sr.as_bytes().len());

    let snn: &str = unsafe { black_box(EMPTY_STR_NN).as_ref() };
    print_usize(snn.len());
    print_bool(snn.is_empty());

    print_usize(black_box(EMPTY_STR_REF).len());
    print_bool(black_box(OPT_STR_SOME).is_some_and(|s| s.is_empty()));
    print_bool(black_box(OPT_STR_NONE).is_none());
    print_bool(black_box(OPT_NN_STR_SOME).is_some_and(|p| unsafe { p.as_ref() }.is_empty()));
    print_bool(black_box(OPT_NN_STR_NONE).is_none());

    let ssr: &str = unsafe { &*black_box(S_EMPTY_STR_RAW.0) };
    print_usize(ssr.len());
    print_usize(black_box(S_EMPTY_STR_REF).len());
    print_bool(black_box(S_OPT_STR).is_some_and(|s| s.is_empty()));

    // Verify zero drops on empty ZST Vec/Box
    print_u32(unsafe { DROPPED });
}
