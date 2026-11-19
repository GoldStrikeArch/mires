//! Aggregates MIR assembles in place: a local that receives its value one field at a time through
//! projections (`_0.0 = ..; _0.1 = ..`) rather than by one assignment of the whole.
//!
//! An aggregate is a JavaScript array or object here, so such a local has no container until
//! something makes one, and a fresh one is owed every time its storage starts over: at the top of
//! the function, at `StorageLive`, and after the value has been moved out of it, since the
//! object that left is now somebody else's (`CONTRACT.md`, "Aggregates assembled in place").
//!
//! rustc writes this shape in the clone shim of a tuple or a closure (`341_clone_shim_aggregates`)
//! and in every body its MIR inliner splices that shim into. This fixture writes the shape
//! directly, in custom MIR, so it does not depend on what the inliner chose:
//!
//! * the return place assembled field by field, for a pair, a triple filled by call
//!   destinations, a struct, an array, an enum (payload, then `SetDiscriminant`) and an enum
//!   variant with no payload (`SetDiscriminant` alone);
//! * a local assembled in a loop and moved into a `Vec` every iteration, with storage markers
//!   (`fill_scoped`) and without them (`fill_bare`): every element must be its own object;
//! * a local assigned whole on one path and assembled on the other (`fill_mixed`), and one
//!   assigned whole before a loop that moves it and assembles it again (`fill_carried`).
//!
//! The vectors are changed after they are filled, and their other elements read back.

#![no_std]
#![no_main]
#![feature(custom_mir, core_intrinsics)]
#![allow(internal_features)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use alloc::vec::Vec;
use core::intrinsics::mir::*;

struct Named {
    label: String,
    count: u32,
}

enum Slot {
    Empty,
    Full(String, u32),
}

/// `t<n>`: a fresh owned string per call.
#[inline(never)]
fn text(n: u32) -> String {
    let mut s = String::from("t");
    let mut digits = [0u8; 10];
    let mut len = 0;
    let mut rest = n;
    loop {
        digits[len] = b'0' + (rest % 10) as u8;
        len += 1;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    while len > 0 {
        len -= 1;
        s.push(digits[len] as char);
    }
    s
}

#[inline(never)]
fn sink(out: &mut Vec<(String, u32)>, item: (String, u32)) {
    out.push(item);
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn pair(s: String, n: u32) -> (String, u32) {
    mir! {
        type RET = (String, u32);
        {
            RET.0 = Move(s);
            RET.1 = n;
            Return()
        }
    }
}

/// The clone shim's own shape: each owning field is a call that returns into it.
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn triple(n: u32) -> (String, u32, String) {
    mir! {
        type RET = (String, u32, String);
        let m: u32;
        {
            Call(RET.0 = text(n), ReturnTo(second), UnwindUnreachable())
        }
        second = {
            RET.1 = n;
            m = n + 1;
            Call(RET.2 = text(m), ReturnTo(done), UnwindUnreachable())
        }
        done = {
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn named(s: String, n: u32) -> Named {
    mir! {
        type RET = Named;
        {
            RET.label = Move(s);
            RET.count = n;
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn array(a: String, b: String) -> [String; 2] {
    mir! {
        type RET = [String; 2];
        let i: usize;
        let j: usize;
        {
            i = 0;
            j = 1;
            RET[i] = Move(a);
            RET[j] = Move(b);
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn full(s: String, n: u32) -> Slot {
    mir! {
        type RET = Slot;
        {
            place!(Field::<String>(Variant(RET, 1), 0)) = Move(s);
            place!(Field::<u32>(Variant(RET, 1), 1)) = n;
            SetDiscriminant(RET, 1);
            Return()
        }
    }
}

#[custom_mir(dialect = "runtime", phase = "optimized")]
fn empty() -> Slot {
    mir! {
        type RET = Slot;
        {
            SetDiscriminant(RET, 0);
            Return()
        }
    }
}

/// One local, assembled and moved out once per iteration, inside its storage markers.
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn fill_scoped(out: &mut Vec<(String, u32)>, n: u32) {
    mir! {
        let i: u32;
        let done: bool;
        let item: (String, u32);
        let target: &mut Vec<(String, u32)>;
        let unit: ();
        {
            i = 0;
            Goto(head)
        }
        head = {
            done = i == n;
            match done {
                true => exit,
                _ => body,
            }
        }
        body = {
            StorageLive(item);
            Call(item.0 = text(i), ReturnTo(rest), UnwindUnreachable())
        }
        rest = {
            item.1 = i;
            target = &mut *out;
            Call(unit = sink(Move(target), Move(item)), ReturnTo(next), UnwindUnreachable())
        }
        next = {
            StorageDead(item);
            i = i + 1;
            Goto(head)
        }
        exit = {
            Return()
        }
    }
}

/// The same with no storage markers, as MIR built without optimization carries it: nothing but
/// the move says that the next iteration starts a new value.
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn fill_bare(out: &mut Vec<(String, u32)>, n: u32) {
    mir! {
        let i: u32;
        let done: bool;
        let item: (String, u32);
        let target: &mut Vec<(String, u32)>;
        let unit: ();
        {
            i = 0;
            Goto(head)
        }
        head = {
            done = i == n;
            match done {
                true => exit,
                _ => body,
            }
        }
        body = {
            Call(item.0 = text(i), ReturnTo(rest), UnwindUnreachable())
        }
        rest = {
            item.1 = i;
            target = &mut *out;
            Call(unit = sink(Move(target), Move(item)), ReturnTo(next), UnwindUnreachable())
        }
        next = {
            i = i + 1;
            Goto(head)
        }
        exit = {
            Return()
        }
    }
}

/// Even iterations assign the local whole, odd ones assemble it.
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn fill_mixed(out: &mut Vec<(String, u32)>, n: u32) {
    mir! {
        let i: u32;
        let done: bool;
        let odd: u32;
        let s: String;
        let item: (String, u32);
        let target: &mut Vec<(String, u32)>;
        let unit: ();
        {
            i = 0;
            Goto(head)
        }
        head = {
            done = i == n;
            match done {
                true => exit,
                _ => pick,
            }
        }
        pick = {
            odd = i & 1;
            match odd {
                0 => whole,
                _ => piece,
            }
        }
        whole = {
            Call(s = text(i), ReturnTo(build), UnwindUnreachable())
        }
        build = {
            item = (Move(s), i);
            Goto(push)
        }
        piece = {
            Call(item.0 = text(i), ReturnTo(piece_rest), UnwindUnreachable())
        }
        piece_rest = {
            item.1 = i;
            Goto(push)
        }
        push = {
            target = &mut *out;
            Call(unit = sink(Move(target), Move(item)), ReturnTo(next), UnwindUnreachable())
        }
        next = {
            i = i + 1;
            Goto(head)
        }
        exit = {
            Return()
        }
    }
}

/// Assigned whole once, before the loop; inside it, moved out and assembled again. This is what
/// `let mut cur = ..; loop { out.push(cur); cur = x.clone(); }` becomes once the tuple's clone
/// shim is inlined.
#[custom_mir(dialect = "runtime", phase = "optimized")]
fn fill_carried(out: &mut Vec<(String, u32)>, n: u32) {
    mir! {
        let i: u32;
        let done: bool;
        let s: String;
        let item: (String, u32);
        let target: &mut Vec<(String, u32)>;
        let unit: ();
        {
            i = 0;
            Call(s = text(100), ReturnTo(start), UnwindUnreachable())
        }
        start = {
            item = (Move(s), 100);
            Goto(head)
        }
        head = {
            done = i == n;
            match done {
                true => exit,
                _ => body,
            }
        }
        body = {
            target = &mut *out;
            Call(unit = sink(Move(target), Move(item)), ReturnTo(again), UnwindUnreachable())
        }
        again = {
            Call(item.0 = text(i), ReturnTo(again_rest), UnwindUnreachable())
        }
        again_rest = {
            item.1 = i;
            i = i + 1;
            Goto(head)
        }
        exit = {
            target = &mut *out;
            Call(unit = sink(Move(target), Move(item)), ReturnTo(ret), UnwindUnreachable())
        }
        ret = {
            Return()
        }
    }
}

fn print_slot(slot: &Slot) {
    match slot {
        Slot::Empty => print_str("empty"),
        Slot::Full(name, count) => {
            print_str(name);
            print_u32(*count);
        }
    }
}

/// Changes the first element, then prints them all.
fn report(mut items: Vec<(String, u32)>) {
    if let Some(first) = items.first_mut() {
        first.0.push('!');
        first.1 += 1000;
    }
    print_usize(items.len());
    for (name, count) in &items {
        print_str(name);
        print_u32(*count);
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let (name, count) = pair(String::from("pair"), 2);
    print_str(&name);
    print_u32(count);

    let (a, n, b) = triple(7);
    print_str(&a);
    print_u32(n);
    print_str(&b);

    let value = named(String::from("named"), 3);
    print_str(&value.label);
    print_u32(value.count);

    let [x, y] = array(String::from("x"), String::from("y"));
    print_str(&x);
    print_str(&y);

    print_slot(&full(String::from("full"), 4));
    print_slot(&empty());

    for fill in [fill_scoped, fill_bare, fill_mixed, fill_carried] {
        let mut out = Vec::new();
        fill(&mut out, 4);
        report(out);
    }
}
