//! Regression test for Seam 5.5 constant-decoding completeness:
//! - `Option::as_slice` constants and `&&[T]` references (`toml`, `coretests::option`, `alloctests::slice`)
//! - `&CStr` (`[i8]`) constants inside structs, slices, and statics (`coretests::ffi`)
//! - `Layout::align_to(3)` (`AlignmentEnum` const-folding at `-O3`, `coretests::alloc`)
//! - Large ZST array constants (`[(); usize::MAX]`, `coretests::iter::adapters::array_chunks`) and
//!   large static byte arrays (`> 64 KiB`).

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::alloc::Layout;
use core::ffi::CStr;

const OPT_SOME_SLICE: &[u32] = Some(44).as_slice();
const OPT_NONE_SLICE: &[u32] = None.as_slice();
const NESTED_SLICE_REF: &&[u32] = &OPT_SOME_SLICE;

const CSTR_LIST: &[&CStr] = &[c"alpha", c"beta"];
static STATIC_CSTR_PAIR: (&CStr, u32) = (c"world", 7);

const HUGE_ZST_SLICE: &[()] = &[(); 100_000];
static BIG_BYTES: [u8; 70_000] = [42u8; 70_000];

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Option::as_slice constants & &&[T]
    print_usize(OPT_SOME_SLICE.len());
    print_u32(OPT_SOME_SLICE[0]);
    print_usize(OPT_NONE_SLICE.len());
    print_u32((*NESTED_SLICE_REF)[0]);

    // &CStr ([i8]) constants
    print_str(CSTR_LIST[0].to_str().unwrap());
    print_str(CSTR_LIST[1].to_str().unwrap());
    print_str(STATIC_CSTR_PAIR.0.to_str().unwrap());
    print_u32(STATIC_CSTR_PAIR.1);

    // AlignmentEnum / Layout::align_to edge cases (exercises -O3 MIR const-prop)
    let layout = Layout::new::<[u8; 2]>();
    print_bool(layout.align_to(3).is_err());
    print_bool(layout.align_to(8).is_ok());
    print_bool(Layout::from_size_align(16, 4).is_ok());

    // Large ZST array & large byte array constants
    let zst_count = [(); usize::MAX].iter().take(5).count();
    print_usize(zst_count);
    print_usize(HUGE_ZST_SLICE.len());
    print_usize(BIG_BYTES.len());
    print_u32(u32::from(BIG_BYTES[69_999]));
}
