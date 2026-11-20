//! Unaligned integer and float reads and writes over byte storage (MIRES-018).
//!
//! A byte pointer cast to a wider scalar reads that scalar's little-endian value from the bytes
//! it names, through a view that aliases them. The view only existed for a byte offset that was
//! a multiple of the scalar's width, so `read_unaligned` at any other offset threw "byte offset N
//! cannot form a u32 scalar view"; that is the load `<[u8]>::is_ascii` makes at the head and the
//! tail of a slice. Here every width is read and written at every offset of a word, over heap and
//! stack bytes, and the cast pointer is moved, compared, cast back and asked for its address.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

extern crate alloc;
use alloc::vec::Vec;
use core::hint::black_box;

/// Every buffer is read as `&[u8]` before any word cast. A heap block written only through raw
/// pointers is still a fresh allocation in this model ("Retyping" in CONTRACT.md), and its
/// first word cast would retype it rather than view its bytes.
#[inline(never)]
fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().map(|b| u32::from(*b)).sum()
}

fn fold(acc: u64, value: u64) -> u64 {
    acc.wrapping_mul(1_000_003).wrapping_add(value)
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    let heap: Vec<u8> = black_box((0..40u8).map(|i| i.wrapping_mul(37).wrapping_add(11)).collect());
    let stack: [u8; 24] = black_box(core::array::from_fn(|i| (i as u8).wrapping_mul(53) ^ 0xa5));
    print_u32(checksum(&heap));
    print_u32(checksum(&stack));

    // Reads of every width at every offset.
    for k in 0..9 {
        let mut acc = 0;
        unsafe {
            let p = heap.as_ptr().add(k);
            acc = fold(acc, u64::from(p.cast::<u16>().read_unaligned()));
            acc = fold(acc, u64::from(p.cast::<u32>().read_unaligned()));
            acc = fold(acc, p.cast::<i32>().read_unaligned() as u64);
            acc = fold(acc, p.cast::<u64>().read_unaligned());
            acc = fold(acc, p.cast::<i64>().read_unaligned() as u64);
            acc = fold(acc, u64::from(p.cast::<f32>().read_unaligned().to_bits()));
            acc = fold(acc, p.cast::<f64>().read_unaligned().to_bits());
            let q = stack.as_ptr().add(k);
            acc = fold(acc, u64::from(q.cast::<u16>().read_unaligned()));
            acc = fold(acc, u64::from(q.cast::<u32>().read_unaligned()));
            acc = fold(acc, q.cast::<u64>().read_unaligned());
        }
        print_u64(acc);
    }

    // A read is the bytes it covers, assembled little endian.
    let mut agree = 0;
    for k in 0..12 {
        let bytes: [u8; 4] = heap[k..k + 4].try_into().unwrap();
        let word = unsafe { heap.as_ptr().add(k).cast::<u32>().read_unaligned() };
        if word == u32::from_le_bytes(bytes) {
            agree += 1;
        }
    }
    print_i32(agree);

    // The cast pointer moves by whole words, compares, casts back and has the byte's address.
    unsafe {
        let base = heap.as_ptr();
        let mut facts = 0;
        for k in 0..4 {
            let byte = base.add(k);
            let word = byte.cast::<u32>();
            if word.cast::<u8>() == byte {
                facts += 1;
            }
            if word.addr() == byte.addr() {
                facts += 10;
            }
            if word.add(1).cast::<u8>() == byte.add(4) {
                facts += 100;
            }
            if word.add(2).cast::<u8>().offset_from(base) == k as isize + 8 {
                facts += 1000;
            }
            if word < word.add(1) && core::ptr::eq(word, byte.cast::<u32>()) {
                facts += 10000;
            }
            let next = word.add(1).read_unaligned();
            if next == u32::from_le_bytes(heap[k + 4..k + 8].try_into().unwrap()) {
                facts += 100000;
            }
        }
        print_i32(facts);
        // Two pointers a byte apart are different places.
        print_bool(core::ptr::eq(base.add(1).cast::<u32>(), base.add(2).cast::<u32>()));
        print_bool(base.add(1).cast::<u32>().addr() + 1 == base.add(2).cast::<u32>().addr());
    }

    // Writes land in the bytes, and a byte write is seen by the next word read.
    let mut buffer: Vec<u8> = black_box((0..24u8).collect());
    print_u32(checksum(&buffer));
    for k in 0..8 {
        unsafe {
            buffer.as_mut_ptr().add(k).cast::<u32>().write_unaligned(0x0102_0304 + k as u32 * 0x1111);
        }
        let mut acc = 0;
        for (index, byte) in buffer.iter().enumerate() {
            acc = fold(acc, (index as u64 + 1) * u64::from(*byte));
        }
        print_u64(acc);
    }
    unsafe {
        let p = buffer.as_mut_ptr().add(5);
        p.cast::<u16>().write_unaligned(0xbeef);
        print_u32(u32::from(buffer[5]) << 8 | u32::from(buffer[6]));
        buffer[7] = 0x42;
        print_u32(p.add(1).cast::<u32>().read_unaligned());
        p.add(4).cast::<f64>().write_unaligned(-1.5);
        print_f64(p.add(4).cast::<f64>().read_unaligned());
        print_i32(i32::from(buffer[16]));
    }

    // A vector that grows after a word was read at an odd offset.
    let mut growing: Vec<u8> = black_box(Vec::from(*b"abcdefg"));
    print_u32(checksum(&growing));
    let first = unsafe { growing.as_ptr().add(3).cast::<u32>().read_unaligned() };
    growing.extend_from_slice(b"hijklmnopq");
    let later = unsafe { growing.as_ptr().add(11).cast::<u32>().read_unaligned() };
    print_u32(first);
    print_u32(later);
}
