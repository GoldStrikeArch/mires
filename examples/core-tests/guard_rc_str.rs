//! The boundary of the unsized heap tail: `Rc<[T]>` is landed and `Rc<str>` is not.
//!
//! A block with an unsized tail is laid out as the header record at element 0 and the tail after
//! it (`CONTRACT.md`, "A struct with an unsized tail"), and a slice tail is a run of elements that
//! run fits. A `str` tail is not: a `str` here is a JavaScript string rather than a buffer, so
//! there is nothing to lay out at index 1 and no element size to lay it out at.
//!
//! The refusal lands where a program would actually notice it, and it lands at compile time
//! because the whole chain is reachable from one line. `Rc<str>::from` is also the shape that
//! shows why this is not simply a missing arm: it builds an `Rc<[u8]>` first and then re-reads the
//! block through `Rc::into_raw` and `Rc::from_raw`, whose `byte_sub(data_offset)` steps backwards
//! across the header boundary. That offset has no place in a model where a pointer names an
//! element rather than an address, so it is on the "Not supported" list on its own account.
//!
//! What this file pins is that the boundary is REPORTED. A backend that quietly answered would
//! hand back a pointer into the middle of a block, and every read through it would be a wrong
//! answer with nothing said anywhere. When the heap `str` tail lands, this test starts failing by
//! succeeding, and the expectation is what has to be rewritten then.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::rc::Rc;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let shared: Rc<str> = Rc::from("statue");
    print_usize(shared.len());
}
