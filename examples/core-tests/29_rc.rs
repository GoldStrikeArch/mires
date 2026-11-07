//! `Rc` and `Arc`: shared ownership over a heap block that counts its owners.
//!
//! An `Rc<T>` is a pointer to an `RcInner<T>` -- `{ strong, weak, value }` -- so everything below
//! is the allocation model already has: one block, a record written into it, and a count read and
//! written through a pointer every clone and every drop. Nothing here is unsized; the pointee is a
//! plain struct and the pointer to it is an ordinary slot.
//!
//! The sections are the whole of the shared-ownership surface a program actually uses: the counts,
//! the unique-access check they gate, pointer identity, the weak half and what it does when the
//! last strong owner goes, drop order, and interior mutability through a clone. `Arc` is the same
//! surface again, because it is the same block with a different count type.
//!
//! # The unsized tail
//!
//! Sections 10 onwards are `Rc<[T]>`, which is a different block shape: a header plus a run of
//! elements, laid out as `[ { strong, weak }, t0, t1, ... ]` with the tail starting at index 1
//! (`CONTRACT.md`, "A struct with an unsized tail"). Everything above has to keep answering the
//! same way with that landed, which is why the two halves share one page.
//!
//! `Rc<str>` is **not** here. Its only constructor goes through `Rc::into_raw` and `Rc::from_raw`,
//! whose `byte_sub(data_offset)` steps across the header boundary -- an offset this model has no
//! place for -- so it stays a refusal and `CONTRACT.md` says so under "Not supported".

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::rc::{Rc, Weak};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

/// A payload whose drop is observable, so "the last owner frees it" is a printed fact rather than
/// an argument.
struct Loud(u32);

impl Drop for Loud {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

struct Point {
    x: i32,
    y: i32,
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. One owner: the value is reachable through the pointer and the count says one.
    let one = Rc::new(41i32);
    print_i32(*one);
    print_usize(Rc::strong_count(&one));

    // 2. A clone is another owner of the SAME block, not a copy of the value.
    let two = Rc::clone(&one);
    print_usize(Rc::strong_count(&one));
    print_i32(*two);
    print_bool(Rc::ptr_eq(&one, &two));

    // A second, independent allocation of the same value is a different block.
    let other = Rc::new(41i32);
    print_bool(Rc::ptr_eq(&one, &other));

    // 3. Dropping an owner lowers the count, and the block outlives it.
    drop(two);
    print_usize(Rc::strong_count(&one));
    print_i32(*one);

    // 4. `get_mut` is the count read as a question: unique or not.
    let mut unique = Rc::new(Point { x: 1, y: 2 });
    match Rc::get_mut(&mut unique) {
        Some(point) => {
            point.x = 10;
            print_bool(true);
        }
        None => print_bool(false),
    }
    let shared = Rc::clone(&unique);
    print_bool(Rc::get_mut(&mut unique).is_none());
    // And the write through the unique reference is what both owners see.
    print_i32(shared.x + shared.y);
    drop(shared);
    print_bool(Rc::get_mut(&mut unique).is_some());

    // 5. A `Weak` does not own the value, so it does not keep it alive.
    let strong = Rc::new(7u32);
    let weak: Weak<u32> = Rc::downgrade(&strong);
    print_usize(Rc::strong_count(&strong));
    print_usize(Rc::weak_count(&strong));
    print_u32(match weak.upgrade() {
        Some(value) => *value,
        None => 0,
    });
    // The upgrade above was a strong owner of its own, and it is gone again.
    print_usize(Rc::strong_count(&strong));
    drop(strong);
    print_bool(weak.upgrade().is_none());

    // 6. Drop order: the payload's glue runs when the LAST strong owner goes, not the first.
    {
        let first = Rc::new(Loud(100));
        let second = Rc::clone(&first);
        drop(first);
        print_str("still alive");
        drop(second);
        print_str("gone");
    }

    // 7. Interior mutability, which is what `Rc` is nearly always carrying: a write through one
    //    clone is a write every clone sees, because there is one block.
    let cell = Rc::new(RefCell::new(5i32));
    let alias = Rc::clone(&cell);
    *alias.borrow_mut() += 3;
    print_i32(*cell.borrow());
    {
        let mut borrowed = cell.borrow_mut();
        *borrowed *= 2;
    }
    print_i32(*alias.borrow());

    // 8. `Arc` is the same block with an atomic count, and answers the same way throughout.
    let arc = Arc::new(Point { x: 3, y: 4 });
    let arc_clone = Arc::clone(&arc);
    print_usize(Arc::strong_count(&arc));
    print_i32(arc_clone.x * arc_clone.y);
    print_bool(Arc::ptr_eq(&arc, &arc_clone));
    drop(arc_clone);
    print_usize(Arc::strong_count(&arc));

    let mut owned = Arc::new(20i32);
    match Arc::get_mut(&mut owned) {
        Some(value) => *value += 2,
        None => print_str("shared"),
    }
    print_i32(*owned);

    let arc_weak = Arc::downgrade(&owned);
    print_i32(match arc_weak.upgrade() {
        Some(value) => *value,
        None => 0,
    });
    drop(owned);
    print_bool(arc_weak.upgrade().is_none());

    // 9. An `Arc` payload with a drop, so the free path is exercised on that side too.
    {
        let loud = Arc::new(Loud(200));
        let echo = Arc::clone(&loud);
        drop(loud);
        print_str("arc alive");
        drop(echo);
        print_str("arc gone");
    }

    // 10. `Rc<[T]>` over a one byte element: the block is reshaped into the header plus the tail,
    //     and every slice operation is the ordinary one over that run of elements.
    let bytes: Rc<[u8]> = Rc::from(&[1u8, 2, 3, 4][..]);
    print_usize(bytes.len());
    print_u32(bytes[0] as u32);
    print_u32(bytes[3] as u32);
    let mut sum = 0u32;
    for byte in bytes.iter() {
        sum += *byte as u32;
    }
    print_u32(sum);

    // The counts work over the tail form exactly as over the sized one, and a clone shares the
    // one block rather than copying the elements.
    let bytes_clone = Rc::clone(&bytes);
    print_usize(Rc::strong_count(&bytes));
    print_bool(Rc::ptr_eq(&bytes, &bytes_clone));
    print_usize(bytes_clone.len());
    drop(bytes_clone);
    print_usize(Rc::strong_count(&bytes));

    // 11. A wider element, so the tail is not the block's byte count.
    let words: Rc<[u32]> = Rc::from(&[10u32, 20, 30][..]);
    print_usize(words.len());
    print_u32(words[1]);
    print_u32(words.iter().sum::<u32>());
    // A subslice of the tail is an ordinary slice into the same block.
    print_usize(words[1..].len());
    print_u32(words[1..].iter().sum::<u32>());

    // 12. An AGGREGATE element, which is the `from_iter` path: `Vec<String>` is moved into a
    //     block of its own rather than copied byte for byte.
    let owned: Vec<String> = vec!["a".to_string(), "bb".to_string(), "ccc".to_string()];
    let names: Rc<[String]> = owned.into();
    print_usize(names.len());
    print_usize(names[2].len());
    print_str(&names[1]);

    // 13. Elements with a `Drop`, which the block's own glue has to run over the tail before the
    //     allocation is freed.
    {
        let louds: Rc<[Loud]> = vec![Loud(300), Loud(400)].into();
        print_usize(louds.len());
    }
    print_str("tail dropped");

    // 14. `Arc<[T]>` is the same block again.
    let shared_words: Arc<[u32]> = Arc::from(&[7u32, 8][..]);
    let shared_clone = Arc::clone(&shared_words);
    print_usize(shared_words.len());
    print_usize(Arc::strong_count(&shared_words));
    print_u32(shared_clone[0] + shared_clone[1]);
    drop(shared_clone);
    print_usize(Arc::strong_count(&shared_words));

    // 15. An empty tail: a length of zero is a real answer and not an absent block.
    let empty: Rc<[u32]> = Rc::from(&[][..]);
    print_usize(empty.len());
    print_bool(empty.is_empty());

    print_str("rc ok");
}
