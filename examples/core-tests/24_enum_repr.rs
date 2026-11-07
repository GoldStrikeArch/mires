//! How `core`'s own enums behave, at every representation the value model gives an enum.
//!
//! The companion of `examples/tests/17_enum_repr.rs`, which pins the same questions at
//! `mini_core` scale over enums written by hand. This one asks them of the enums `core` actually
//! ships, because those are the ones the rest of the program is built out of:
//!
//! * `Ordering` and `mem::Alignment` are fieldless with an integer `repr`, so a value of one is
//!   nothing but its discriminant and a `transmute` to that integer is a move;
//! * `Option<T>` is the mixed shape, and its layout is a *niche* for a `&T`, a `NonZero` or a
//!   `char` payload, which is a second tag encoding the backend has to decode;
//! * `Option::take`, `replace` and `insert` all write a whole new value through a `&mut
//!   Option<T>`, including over a `None` that carries no payload. Those writes are what a
//!   representation of the empty variant has to survive, and they are on the path of every
//!   iterator adapter in `core`.
//!
//! `mem::discriminant` is asked too: it is the one public way to observe a discriminant without
//! matching on it, so it has to keep agreeing with `==` on the variants themselves.

#![no_std]
#![no_main]

use core::cmp::Ordering;
use core::mem;
use core::num::NonZero;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Low,
    Mid,
    High,
}

/// An or-pattern over a fieldless enum, which is the multi-value test the switch lowering spells.
fn is_edge(level: Level) -> bool {
    match level {
        Level::Low | Level::High => true,
        Level::Mid => false,
    }
}

fn level_value(level: Level) -> i32 {
    match level {
        Level::Low => 1,
        Level::Mid => 2,
        Level::High => 3,
    }
}

fn ordering_value(ordering: Ordering) -> i32 {
    match ordering {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// A whole-value write through a `&mut Option<T>` where the payload is niched: the empty variant
/// has no payload to hold the tag, so writing one over a `Some` and back is the round trip that
/// proves the encoding is total.
fn swap_ref<'a>(slot: &mut Option<&'a i32>, value: Option<&'a i32>) -> Option<&'a i32> {
    mem::replace(slot, value)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // A derived fieldless enum: matching, or-patterns, equality and ordering.
    print_i32(level_value(Level::Low));
    print_i32(level_value(Level::High));
    print_bool(is_edge(Level::Low));
    print_bool(is_edge(Level::Mid));
    print_bool(Level::Mid == Level::Mid);
    print_bool(Level::Mid == Level::High);

    // `mem::discriminant` agrees with `==` on the variants.
    print_bool(mem::discriminant(&Level::High) == mem::discriminant(&Level::High));
    print_bool(mem::discriminant(&Level::High) == mem::discriminant(&Level::Low));

    // `Ordering`, which is `#[repr(i8)]` with a negative discriminant.
    print_i32(ordering_value(3i32.cmp(&7)));
    print_i32(ordering_value(7i32.cmp(&7)));
    print_i32(ordering_value(9i32.cmp(&7)));
    print_i32(Ordering::Less as i32);
    print_i32(Ordering::Greater as i32);
    print_bool(Ordering::Less.is_lt());
    print_i32(ordering_value(Ordering::Less.reverse()));

    // `Ordering` moved through a `transmute` to the integer it is.
    let raw: i8 = unsafe { mem::transmute(Ordering::Greater) };
    print_i32(raw as i32);
    let back: Ordering = unsafe { mem::transmute(-1i8) };
    print_i32(ordering_value(back));

    // `mem::Alignment` is a fieldless `#[repr(usize)]` enum behind a transparent wrapper, and
    // every allocation reads it as a plain number.
    print_usize(mem::align_of::<u32>());
    print_usize(mem::size_of::<Option<&i32>>());

    // A niched `Option<&T>`: built, matched, and unwrapped.
    let target = 41;
    let some_ref: Option<&i32> = Some(&target);
    let none_ref: Option<&i32> = None;
    print_bool(some_ref.is_some());
    print_bool(none_ref.is_none());
    print_i32(*some_ref.unwrap());
    print_i32(*none_ref.unwrap_or(&-1));

    // Whole-value writes through a `&mut Option<&T>`, in both directions.
    let other = 7;
    let mut slot: Option<&i32> = None;
    let previous = swap_ref(&mut slot, Some(&other));
    print_bool(previous.is_none());
    print_i32(*slot.unwrap());
    let previous = swap_ref(&mut slot, None);
    print_i32(*previous.unwrap());
    print_bool(slot.is_none());

    // `take`, `replace`, `insert` and `get_or_insert`: the `&mut Option<T>` surface.
    let mut owned: Option<i32> = Some(5);
    print_i32(owned.take().unwrap());
    print_bool(owned.is_none());
    print_i32(*owned.get_or_insert(6));
    print_i32(owned.replace(8).unwrap());
    print_i32(owned.unwrap());
    print_i32(*owned.insert(9));
    print_i32(owned.unwrap());

    // A niched `Option<NonZero<u32>>`, which is how `NonZero::new` is written.
    print_bool(NonZero::<u32>::new(0).is_none());
    print_u32(NonZero::<u32>::new(12).unwrap().get());
    print_u32(NonZero::<u32>::new(u32::MAX).unwrap().get());

    // A niched `Option<char>`, whose niche is a value no `char` may hold.
    let some_char: Option<char> = Some('q');
    let none_char: Option<char> = None;
    print_bool(some_char.is_some());
    print_bool(none_char.is_none());
    print_i32(some_char.unwrap() as i32);

    // An `Option` inside an aggregate, copied rather than aliased.
    let pair = [Some(1i32), None];
    let mut copy = pair;
    copy[1] = Some(2);
    print_i32(pair[0].unwrap_or(-1));
    print_i32(pair[1].unwrap_or(-1));
    print_i32(copy[1].unwrap_or(-1));

    // `Result`, whose every variant carries a payload.
    let ok: Result<i32, &str> = Ok(3);
    let err: Result<i32, &str> = Err("no");
    print_i32(ok.unwrap_or(-1));
    print_i32(err.unwrap_or(-1));
    print_str(err.unwrap_err());

    print_str("24_enum_repr ok");
}
