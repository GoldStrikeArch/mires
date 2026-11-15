//! Exact `Enumerate<Take|Skip<SliceIter<u8>>>` cursor lowering and its hard declines.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Lane {
    high: u32,
    low: u32,
}

impl Lane {
    fn xor_byte(&mut self, value: u8, shift: usize) {
        if shift < 32 {
            self.low ^= u32::from(value) << shift;
        } else {
            self.high ^= u32::from(value) << (shift - 32);
        }
    }
}

#[inline(never)]
fn tail_checksum(values: &[u8], start: usize, split: usize) -> u32 {
    let tail = &values[start..];
    let mut first = Lane { high: 0, low: 0 };
    let mut second = Lane { high: 0, low: 0 };
    for (index, &value) in tail.iter().take(split).enumerate() {
        first.xor_byte(value, index * 8);
    }
    for (index, &value) in tail.iter().skip(split).enumerate() {
        second.xor_byte(value, index * 8);
    }
    first.high ^ first.low ^ second.high ^ second.low
}

// A different element width must not enter the byte-only rule.
#[inline(never)]
fn wide_checksum(values: &[u16], limit: usize) -> usize {
    let mut checksum = 0;
    for (index, &value) in values.iter().take(limit).enumerate() {
        checksum += (index + 1) * value as usize;
    }
    checksum
}

// A signed byte has the same layout, but is outside this tranche's exact `u8` contract.
#[inline(never)]
fn signed_checksum(values: &[i8], limit: usize) -> i32 {
    let mut checksum = 0;
    for (index, &value) in values.iter().take(limit).enumerate() {
        checksum += (index as i32 + 1) * i32::from(value);
    }
    checksum
}

// Adapter order is semantic: this is `Take<Enumerate<SliceIter<_>>>`, not the accepted type.
#[inline(never)]
fn other_order(values: &[u8], limit: usize) -> usize {
    let mut checksum = 0;
    for (index, &value) in values.iter().enumerate().take(limit) {
        checksum += (index + 1) * value as usize;
    }
    checksum
}

// Observing an adapter after advancing it makes its state live and must decline scalarization.
#[inline(never)]
fn observed_state(values: &[u8], limit: usize) -> usize {
    let mut iterator = values.iter().take(limit).enumerate();
    let first = iterator
        .next()
        .map(|(index, &value)| index + value as usize)
        .unwrap_or(0);
    first + iterator.len()
}

// A user-visible early exit exercises the shared empty-exit proof.
#[inline(never)]
fn early_break(values: &[u8], limit: usize, stop: u8) -> usize {
    let mut checksum = 0;
    for (index, &value) in values.iter().take(limit).enumerate() {
        if value == stop {
            break;
        }
        checksum += (index + 1) * value as usize;
    }
    checksum
}

// Unused iterator items fold `Take::next`'s borrow away; an unrelated `&u8`
// deref in the body must never be confused for the slice item.
#[inline(never)]
fn untainted_deref(values: &[u8], limit: usize, other: *const u8, stop: u8) -> usize {
    let mut checksum = 0;
    for (index, _) in values.iter().take(limit).enumerate() {
        let value = unsafe { *other };
        if value == stop {
            break;
        }
        checksum += (index + 1) * value as usize;
    }
    checksum
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let values = [
        1_u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        20,
    ];
    for split in [0, 2, 8, 20, 30] {
        print_u32(tail_checksum(&values, 0, split));
    }
    print_u32(tail_checksum(&values, 3, 8));
    print_u32(tail_checksum(&[], 0, 8));

    print_usize(wide_checksum(&[1000, 2000, 3000], 2));
    print_i32(signed_checksum(&[-2, 3, -4], 3));
    print_usize(other_order(&values, 3));
    print_usize(observed_state(&values, 4));
    print_usize(early_break(&values, 10, 5));
    let sentinel = 100_u8;
    print_usize(untainted_deref(&values, 3, &sentinel as *const u8, 255));
}
