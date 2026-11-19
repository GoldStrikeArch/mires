//! `Box<[T]>` built by `From<&[T]>`, which `alloc` writes as `Box::clone_from_ref`: allocate the
//! bytes, clone the elements into them, and rebuild the fat pointer with
//! `ptr.with_metadata_of(src)`.
//!
//! That last step is `ptr::from_raw_parts(ptr as *const (), len)`. The data half arrives erased to
//! a pointer to `()`, which says nothing about what the value is, and the JIR lowering read the
//! erased type instead of the value: it took the shortcut meant for a reference to an array, whose
//! value *is* the buffer, and built a slice whose buffer was the pointer record. Every element read
//! came back wrong and dropping the box refused to free it ("this pointer does not name a live
//! heap block"). `into_boxed_slice` never took that path, which is why it worked.
//!
//! Each box is read, written through, compared with one built the other way, and dropped; the
//! `Drop` element counts its drops so a double or missing one shows.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

#[derive(Clone)]
struct Counted<'a> {
    value: u32,
    drops: &'a Cell<u32>,
}

impl Drop for Counted<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[inline(never)]
fn boxed_units(units: &[u16], how: u32) -> Box<[u16]> {
    match how {
        0 => units.into(),
        1 => units[units.len().min(1)..].into(),
        _ => units.to_vec().into_boxed_slice(),
    }
}

#[inline(never)]
fn boxed_bytes(bytes: &[u8]) -> Box<[u8]> {
    Box::from(bytes)
}

#[inline(never)]
fn boxed_words(words: &mut [u32]) -> Box<[u32]> {
    Box::from(words)
}

#[inline(never)]
fn boxed_strings(strings: &[String]) -> Box<[String]> {
    strings.into()
}

#[inline(never)]
fn boxed_counted<'a>(items: &[Counted<'a>]) -> Box<[Counted<'a>]> {
    items.into()
}

fn print_units(units: &[u16]) {
    print_usize(units.len());
    for unit in units {
        print_u32(*unit as u32);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let units: Vec<u16> = "hello".encode_utf16().collect();
    for how in 0..3 {
        let mut boxed = boxed_units(&units, how);
        print_units(&boxed);
        if let Some(first) = boxed.first_mut() {
            *first += 1;
        }
        print_u32(boxed[0] as u32);
        drop(boxed);
    }
    print_bool(*boxed_units(&units, 0) == *boxed_units(&units, 2));
    print_units(&units);

    // Nothing to allocate: the dangling case of the same path.
    let empty = boxed_units(&[], 0);
    print_usize(empty.len());

    let bytes = boxed_bytes(b"bytes");
    print_usize(bytes.len());
    print_u32(bytes[4] as u32);

    let mut source = [7u32, 8, 9];
    let mut words = boxed_words(&mut source);
    words[2] = 90;
    print_u32(words[2]);
    print_u32(source[2]);
    print_u32(words.iter().sum());

    let strings = [String::from("one"), String::from("two")];
    let mut boxed = boxed_strings(&strings);
    boxed[1].push('!');
    print_str(&boxed[0]);
    print_str(&boxed[1]);
    print_str(&strings[1]);
    drop(boxed);

    let drops = Cell::new(0);
    let items = [
        Counted { value: 1, drops: &drops },
        Counted { value: 2, drops: &drops },
        Counted { value: 3, drops: &drops },
    ];
    let boxed = boxed_counted(&items);
    print_u32(boxed.iter().map(|item| item.value).sum());
    print_u32(drops.get());
    drop(boxed);
    print_u32(drops.get());
    drop(items);
    print_u32(drops.get());
}
