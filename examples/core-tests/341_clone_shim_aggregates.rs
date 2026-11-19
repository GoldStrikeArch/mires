//! `Clone` of the aggregates rustc writes the clone itself for: a tuple or a closure whose fields
//! are not all `Copy`.
//!
//! rustc builds that `clone` as a shim that clones each field straight into the return place,
//! `_0.0 = <A as Clone>::clone(&(*_1).0); _0.1 = ..`, and never assigns `_0` whole. Here an
//! aggregate is a JavaScript array or object, so that return place has to be given its container
//! before the first field lands in it (`CONTRACT.md`, "Aggregates assembled in place"). Before
//! that, every such clone threw `Cannot set properties of undefined`, and so did every
//! `Vec<(String, _)>::clone`, which clones element by element.
//!
//! Every clone below is changed afterwards, and the original read back, so a clone that shared
//! storage with its source would show. Derived clones of a struct and an enum, and the library
//! clone of an array, are the shapes beside it that build their value whole; they are here so the
//! class is pinned from both sides.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone)]
struct Labelled {
    label: String,
    weight: u32,
}

#[derive(Clone)]
enum Entry {
    Missing,
    Present(String, u32),
}

#[inline(never)]
fn clone_pair(pair: &(String, usize)) -> (String, usize) {
    pair.clone()
}

#[inline(never)]
fn clone_triple(triple: &(String, u32, Vec<u8>)) -> (String, u32, Vec<u8>) {
    triple.clone()
}

#[inline(never)]
fn clone_nested(nested: &((String, u8), u16)) -> ((String, u8), u16) {
    nested.clone()
}

#[inline(never)]
fn clone_with_unit(value: &((), String)) -> ((), String) {
    value.clone()
}

#[inline(never)]
fn clone_list(list: &Vec<(String, u32)>) -> Vec<(String, u32)> {
    list.clone()
}

#[inline(never)]
fn clone_labelled(value: &Labelled) -> Labelled {
    value.clone()
}

#[inline(never)]
fn clone_entry(value: &Entry) -> Entry {
    value.clone()
}

#[inline(never)]
fn clone_array(value: &[String; 2]) -> [String; 2] {
    value.clone()
}

#[inline(never)]
fn clone_maybe(value: &Option<(String, u32)>) -> Option<(String, u32)> {
    value.clone()
}

/// A closure that owns a `String` is `Clone`, and its clone is the same field-by-field shim.
#[inline(never)]
fn clone_closure<F: Fn() -> usize + Clone>(f: &F) -> F {
    f.clone()
}

fn print_entry(entry: &Entry) {
    match entry {
        Entry::Missing => print_str("missing"),
        Entry::Present(name, count) => {
            print_str(name);
            print_u32(*count);
        }
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // A pair, the shape the library matrix found.
    let pair = (String::from("hello"), 5usize);
    let mut copy = clone_pair(&pair);
    copy.0.push_str(" world");
    copy.1 += 1;
    print_str(&copy.0);
    print_usize(copy.1);
    print_str(&pair.0);
    print_usize(pair.1);

    // Three fields, two of them owning.
    let triple = (String::from("abc"), 7u32, vec![1u8, 2, 3]);
    let mut copy = clone_triple(&triple);
    copy.2.push(4);
    copy.0.push('d');
    print_str(&copy.0);
    print_u32(copy.1);
    print_usize(copy.2.len());
    print_usize(triple.2.len());
    print_str(&triple.0);

    // A tuple inside a tuple: the inner one is assembled by its own shim.
    let nested = ((String::from("in"), 1u8), 2u16);
    let mut copy = clone_nested(&nested);
    (copy.0).0.push_str("ner");
    print_str(&(copy.0).0);
    print_u32((copy.0).1 as u32);
    print_u32(copy.1 as u32);
    print_str(&(nested.0).0);

    // A zero-sized field stores nothing, so the container has a hole where it would be.
    let with_unit = ((), String::from("unit"));
    let copy = clone_with_unit(&with_unit);
    print_str(&copy.1);

    // `Vec<(String, u32)>::clone` clones each element through the tuple shim: every clone has to
    // be a container of its own.
    let list = vec![
        (String::from("a"), 1u32),
        (String::from("b"), 2),
        (String::from("c"), 3),
    ];
    let mut copy = clone_list(&list);
    copy[0].0.push('!');
    copy[2].1 = 30;
    for (name, count) in &copy {
        print_str(name);
        print_u32(*count);
    }
    for (name, count) in &list {
        print_str(name);
        print_u32(*count);
    }

    // The same, one clone per iteration into a vector that keeps them all.
    let mut kept: Vec<(String, u32)> = Vec::new();
    for item in &list {
        kept.push(item.clone());
    }
    kept[1].0.push('?');
    for (name, count) in &kept {
        print_str(name);
        print_u32(*count);
    }
    print_str(&list[1].0);

    // An optional tuple: the derived `Option` clone hands the payload to the tuple shim.
    let maybe = Some((String::from("some"), 9u32));
    let copy = clone_maybe(&maybe);
    if let Some((name, count)) = &copy {
        print_str(name);
        print_u32(*count);
    }
    print_bool(clone_maybe(&None).is_none());

    // A closure that owns its capture.
    let captured = String::from("captured");
    let measure = move || captured.len();
    let copy = clone_closure(&measure);
    print_usize(copy());
    print_usize(measure());

    // The shapes beside it, which build the clone whole: a derived struct, a derived enum, and the
    // library clone of an array.
    let labelled = Labelled {
        label: String::from("label"),
        weight: 4,
    };
    let mut copy = clone_labelled(&labelled);
    copy.label.push('s');
    print_str(&copy.label);
    print_u32(copy.weight);
    print_str(&labelled.label);

    print_entry(&clone_entry(&Entry::Present(String::from("entry"), 11)));
    print_entry(&clone_entry(&Entry::Missing));

    let array = [String::from("x"), String::from("y")];
    let mut copy = clone_array(&array);
    copy[1].push('z');
    print_str(&copy[0]);
    print_str(&copy[1]);
    print_str(&array[1]);
}
