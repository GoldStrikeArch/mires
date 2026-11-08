//! The transactional commit's rollback (Phase 4, D4): under `RCGJS_REPR_FAULT=verify` every
//! region's commit-time verification fails, the regions decline atomically
//! (`version.carrier-commit-rollback` in the sidecar's `repr` block), the originals are kept,
//! and the printed program is byte for byte the one a build without the
//! `native-representations` pass prints — which is what `scripts/repr-rollback-test.sh` holds
//! it to. Without the fault this little builder commits, and stdout is identical either way.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let mut s = String::with_capacity(4);
    s.push_str("ok");
    print_str(&s);
    print_usize(s.len());
    print_usize(s.capacity());
}
