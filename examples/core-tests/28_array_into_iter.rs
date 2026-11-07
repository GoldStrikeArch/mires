//! An array iterated BY VALUE, which is a reference to a struct with an unsized tail.
//!
//! `for x in [a, b, c]` does not iterate a slice. `<[T; N] as IntoIterator>::into_iter` builds an
//! `array::IntoIter<T, N>`, which holds a `ManuallyDrop<PolymorphicIter<[MaybeUninit<T>; N]>>` and
//! unsizes it to `&mut PolymorphicIter<[MaybeUninit<T>]>` for every operation:
//!
//! ```text
//! struct PolymorphicIter<DATA: ?Sized> { alive: IndexRange, data: DATA }
//! ```
//!
//! That is a struct whose LAST field is unsized, and a reference to one is a fat pointer of its
//! own: `{ ptr, meta }`, the record and the length of its tail, the same shape `&dyn Trait` has
//! (CONTRACT.md, "Pointers"). Without it the coercion collapsed to the lockstep tails and handed
//! back a fat slice over the whole record, while the dereference on the other side read that
//! record as the struct: `next()` asked a `{ buf, off, len }` for `.alive.end` and got a
//! `TypeError`.
//!
//! What is exercised here:
//!
//! * `for s in ["a", "b", "c", "d"]` -- the exact shape a view's `for` over a literal array takes,
//!   and the one the showcase island hit;
//! * a `[u32; N]` by value, so the element is a number rather than a string;
//! * `.rev()` and `nth`, which move `alive.end` and `alive.start` respectively, so both ends of
//!   the alive range are read and written through the fat pointer;
//! * an element type with a `Drop`, whose glue runs over `data[alive]` through the same reference.
//!
//! The heap half of the same representation -- `Rc<[T]>`, whose block is a header plus an unsized
//! tail -- is a separate landing and still a zombie.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

/// An element whose drop is observable, so that what the iterator did or did not consume can be
/// counted rather than argued about.
struct Loud(u32);

impl Drop for Loud {
    fn drop(&mut self) {
        unsafe { DROPPED += self.0 }
    }
}

static mut DROPPED: u32 = 0;

fn dropped() -> u32 {
    unsafe { DROPPED }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. The showcase shape: a literal array of `&str`, iterated by value.
    for s in ["a", "b", "c", "d"] {
        print_str(s);
    }

    // 2. The same array through `into_iter`, counted and summed as numbers.
    let mut total = 0u32;
    let mut count = 0u32;
    for n in [10u32, 20, 30, 40, 50] {
        total += n;
        count += 1;
    }
    print_u32(total);
    print_u32(count);

    // 3. `len` and `size_hint` off the unsized reference, before anything is taken.
    let iter = [1u32, 2, 3, 4, 5, 6].into_iter();
    print_usize(iter.len());
    let (low, high) = iter.size_hint();
    print_usize(low);
    print_usize(match high {
        Some(high) => high,
        None => 0,
    });

    // 4. `alive.end`: `rev()` yields from the back, so every step writes the far end of the range.
    let mut back = 0u32;
    for n in [1u32, 2, 3, 4].into_iter().rev() {
        back = back * 10 + n;
    }
    print_u32(back);

    // 5. `alive.start`: `nth` skips forward, and the iterator keeps going from where it stopped.
    let mut skip = [1u32, 2, 3, 4, 5, 6].into_iter();
    print_u32(match skip.nth(2) {
        Some(n) => n,
        None => 0,
    });
    print_u32(match skip.next() {
        Some(n) => n,
        None => 0,
    });
    print_usize(skip.len());

    // 6. Both ends at once, which is what `DoubleEndedIterator` on a partly drained iterator is.
    let mut ends = [1u32, 2, 3, 4, 5].into_iter();
    print_u32(match ends.next() {
        Some(n) => n,
        None => 0,
    });
    print_u32(match ends.next_back() {
        Some(n) => n,
        None => 0,
    });
    print_usize(ends.len());

    // 7. A `Drop` element, fully consumed: every element is moved out, so the iterator's own glue
    //    has nothing left to drop and each `Loud` dies at the end of its loop body.
    for loud in [Loud(1), Loud(2), Loud(4)] {
        let _ = loud;
    }
    print_u32(dropped());

    // 8. A `Drop` element, PARTLY consumed: the iterator is dropped holding the rest, and its glue
    //    drops `data[alive]` through the unsized reference. 8 is taken and dies in the body; 16
    //    and 32 die with the iterator.
    {
        let mut partial = [Loud(8), Loud(16), Loud(32)].into_iter();
        let taken = partial.next();
        print_bool(taken.is_some());
    }
    print_u32(dropped());

    print_str("array into_iter ok");
}
