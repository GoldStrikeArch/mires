//! The D2 region cut, end to end: an app `Vec` and an app `String` built, grown and read
//! through producer-tagged standard calls (`CallSemantic::Standard`).
//!
//! Before Phase 5 the value graph must-equal-joined these values into the untagged alloc-crate
//! bodies behind the tagged wrappers (`push_mut`, `RawVec` internals), whose raw views poisoned
//! the whole region. With the cut, the caller's values carry the registry's slot requirements
//! directly, the untagged internals form their own declining regions, and both app regions
//! commit — the internals stop being referenced by the committed native op bodies and drop from
//! the output. The expected stdout is mode-identical: baseline and jir, `off`, `report` and `on`
//! must print exactly this.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use alloc::vec::Vec;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // The vec region: construction, growth, indexing, removal, capacity.
    let mut values: Vec<u32> = Vec::new();
    values.push(3);
    values.push(5);
    values.push(8);
    print_usize(values.len());
    print_u32(values[0]);
    print_u32(values.remove(1));
    print_u32(values[1]);
    print_bool(values.capacity() >= 2);

    // The string region: construction, growth, length, a shared read, clear.
    let mut text = String::new();
    text.push('r');
    text.push_str("ust");
    print_usize(text.len());
    print_str(&text);
    text.clear();
    print_usize(text.len());
}
