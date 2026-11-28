//! Selective `ArrayBuffer` + `TypedArray` / `DataView` views for cross-granularity POD buffers
//! (`&[u32] <-> &[u8]`, aligned & unaligned reads/writes) and large `TypedArray` `alloc_zeroed`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::vec;
use core::hint::black_box;

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. Large `vec![0u8; 16_000_000]` (O(1) OS-zeroed Uint8Array backing!)
    let mut big_bytes = black_box(vec![0u8; 16_000_000]);
    print_usize(big_bytes.len());
    print_u32(u32::from(big_bytes[0]));
    print_u32(u32::from(big_bytes[15_999_999]));
    big_bytes[12_345_678] = 0xab;
    print_u32(u32::from(big_bytes[12_345_678]));
    drop(big_bytes);

    // 2. Large `vec![0u32; 1_000_000]` (zero-copy Uint32Array over OS-zeroed ArrayBuffer!)
    let mut big_words = black_box(vec![0u32; 1_000_000]);
    print_usize(big_words.len());
    print_u32(big_words[0]);
    print_u32(big_words[999_999]);
    big_words[500_000] = 0x1122_3344;
    print_u32(big_words[500_000]);

    // Reinterpret `big_words` as `&mut [u8]` (zero-copy Uint8Array sharing ArrayBuffer!)
    unsafe {
        let bp = big_words.as_mut_ptr() as *mut u8;
        let base = 500_000 * 4;
        print_u32(u32::from(*bp.add(base)));
        print_u32(u32::from(*bp.add(base + 3)));
        *bp.add(base + 1) = 0xee;
    }
    print_u32(big_words[500_000]);
    drop(big_words);

    // 3. Cross-width POD views on `vec![0u8; 128]`: aligned & unaligned u32, u16, f32, u64
    let mut buf = black_box(vec![0u8; 128]);
    unsafe {
        // Aligned u32 write at byte offset 0
        let w0 = buf.as_mut_ptr() as *mut u32;
        *w0 = 0x1234_5678;
        print_u32(u32::from(buf[0]));
        print_u32(u32::from(buf[3]));

        // Unaligned u32 read & write at byte offset 1 (DataView-backed view!)
        let w1 = buf.as_mut_ptr().add(1) as *mut u32;
        print_u32(*w1);
        *w1 = 0x89ab_cdef;
        print_u32(u32::from(buf[1]));
        print_u32(u32::from(buf[4]));
        print_u32(*w0);

        // Aligned u16 view at byte offset 8
        let h = buf.as_mut_ptr().add(8) as *mut u16;
        *h = 0xbeef;
        print_u32(u32::from(buf[8]));
        print_u32(u32::from(buf[9]));

        // Unaligned u64 write & read at byte offset 17
        let q = buf.as_mut_ptr().add(17) as *mut u64;
        *q = 0x0102_0304_0506_0708u64;
        print_u64(*q);
        print_u32(u32::from(buf[17]));
        print_u32(u32::from(buf[24]));
    }

    // 4. Cross-width `&mut [u32]` -> `*mut u16` and `*mut f32` on same ArrayBuffer
    let mut u32s = black_box(vec![0u32; 32]);
    u32s[0] = 0x3f80_0000; // 1.0f32
    unsafe {
        let fp = u32s.as_mut_ptr() as *mut f32;
        print_f64(f64::from(*fp));
        *fp = 2.5f32;
        print_u32(u32s[0]);

        let hp = u32s.as_mut_ptr() as *mut u16;
        print_u32(u32::from(*hp.add(0)));
        print_u32(u32::from(*hp.add(1)));
    }

    // 5. `vec![0i8; 128]`, `vec![false; 128]`, and `realloc` growth on zeroed TypedArray buffers
    let mut signed = black_box(vec![0i8; 128]);
    signed[0] = -42;
    signed.push(-1);
    print_i32(i32::from(signed[0]));
    print_i32(i32::from(signed[128]));

    let mut flags = black_box(vec![false; 128]);
    print_bool(flags[0]);
    flags[127] = true;
    print_bool(flags[127]);

    let mut grow_u32 = black_box(vec![0u32; 64]);
    grow_u32[63] = 777;
    grow_u32.push(888);
    print_u32(grow_u32[63]);
    print_u32(grow_u32[64]);
}
