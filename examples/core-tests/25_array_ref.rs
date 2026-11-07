//! A reference to a fixed size array, taken from a pointer that names the array's *elements*.
//!
//! `&*p` where the pointee is `[E; N]` has to be an array object, because that is what a reference
//! to an aggregate is. Which JavaScript value that is depends on what the pointer is:
//!
//! * a plain slot holds the whole array as its one element, so the element is the array;
//! * a **window** slot, which an `as *mut [E; N]` cast produces, and a **slice** record, which
//!   `<&mut [T] as TryInto<&mut [T; N]>>` leaves its `len` on, both name `N` consecutive elements
//!   of a longer buffer. There is no array object at that offset, so one is made whose elements
//!   are accessors onto those places.
//!
//! Only the first of the three used to be emitted, for all three, which silently handed the callee
//! ONE ELEMENT where an array belonged. Every case here failed with
//! `TypeError: Cannot set properties of undefined` before `__rt.array_ref` asked the record.
//!
//! The shape is `itoa`'s, which is what made it matter: `itoa::Buffer` is a
//! `[MaybeUninit<u8>; 40]`, `Buffer::format` casts a pointer to it down to a shorter array, and the
//! integer writers convert `&mut buf[1..]` back to a fixed size array at a NON-ZERO offset. Both
//! reach the aliasing case, which is why `serde_json` could parse integers but not print them.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::mem::MaybeUninit;

/// `itoa::Buffer`: room for the longest answer, written back to front.
struct Buffer {
    bytes: [MaybeUninit<u8>; 24],
}

impl Buffer {
    fn new() -> Buffer {
        Buffer { bytes: [MaybeUninit::<u8>::uninit(); 24] }
    }

    /// `itoa::Buffer::format`: the cast down to the shorter array the writer takes, then the
    /// decimal digits, then the written part as a `str`.
    fn format(&mut self, value: u32) -> &str {
        let at = {
            let short = unsafe {
                &mut *(&mut self.bytes as *mut [MaybeUninit<u8>; 24] as *mut [MaybeUninit<u8>; 10])
            };
            write_decimal(short, value)
        };
        let written = &self.bytes[at..10];
        // Every byte from `at` to 10 was written by `write_decimal`, so the slice is initialized.
        let bytes = unsafe { &*(written as *const [MaybeUninit<u8>] as *const [u8]) };
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }
}

/// Writes `value`'s decimal digits into the end of `buf` and answers where they start.
fn write_decimal(buf: &mut [MaybeUninit<u8>; 10], mut value: u32) -> usize {
    let mut at = 10;
    loop {
        at -= 1;
        buf[at] = MaybeUninit::new(b'0' + (value % 10) as u8);
        value /= 10;
        if value == 0 || at == 0 {
            return at;
        }
    }
}

/// The `try_into` shape: a sub-slice back to a fixed size array reference, at offset 1.
fn through_try_into(all: &mut [MaybeUninit<u8>]) -> u32 {
    let short: &mut [MaybeUninit<u8>; 10] = (&mut all[1..11]).try_into().unwrap();
    short[0] = MaybeUninit::new(7);
    short[9] = MaybeUninit::new(9);
    unsafe { short[0].assume_init() as u32 + short[9].assume_init() as u32 }
}

/// A reference taken from a plain slot, which is the case that always worked.
fn through_plain_slot(buf: &mut [MaybeUninit<u8>; 10]) -> u32 {
    let same = unsafe { &mut *(buf as *mut [MaybeUninit<u8>; 10]) };
    same[3] = MaybeUninit::new(4);
    unsafe { same[3].assume_init() as u32 }
}

#[no_mangle]
pub fn rust_entry() {
    let mut buffer = Buffer::new();
    print_str(buffer.format(0));
    print_str(buffer.format(7));
    print_str(buffer.format(4321));
    print_str(buffer.format(4294967295));

    // The write has to land in the caller's own buffer, not in a copy of it.
    let mut all = [MaybeUninit::<u8>::uninit(); 16];
    print_u32(through_try_into(&mut all));
    print_u32(unsafe { all[1].assume_init() } as u32);
    print_u32(unsafe { all[10].assume_init() } as u32);

    let mut short = [MaybeUninit::<u8>::uninit(); 10];
    print_u32(through_plain_slot(&mut short));
    print_u32(unsafe { short[3].assume_init() } as u32);
}
