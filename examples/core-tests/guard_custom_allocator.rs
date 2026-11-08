//! A custom allocator pins its values to the baseline representation (Phase 5, D10f).
//!
//! `Vec<i32, Arena>` names an allocator type other than `Global`, and a custom allocator can
//! observe every allocation — the size it is asked for, the address it hands back — so converting
//! the vec to a native carrier would change what the allocator sees. The backend records
//! `TypeMeta.allocator = Custom` for the type (`backend/src/late_ir.rs::allocator_identity`), the
//! planner turns that into a unary INF term (`analysis/legality.rs`), and the region declines
//! `boundary.custom-allocator` in the lane sidecar. That decline is the subject of this guard —
//! `scripts/allocator-guard-test.sh` asserts it — and the program itself must keep printing the
//! same thing in every optimizer mode, because a decline is a representation decision, never a
//! behavior change.
//!
//! `Arena` delegates every call to `Global`: the point is the allocator's *type identity*, not a
//! different memory source, and delegation keeps the runtime on the ordinary heap model.

#![no_std]
#![no_main]
#![feature(allocator_api)]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::alloc::Global;
use alloc::vec::Vec;
use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

/// A distinct allocator type that hands every request to `Global`.
struct Arena;

unsafe impl Allocator for Arena {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        Global.allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe { Global.deallocate(ptr, layout) }
    }

    // Delegated rather than defaulted: the trait's default `grow` allocates a new block and
    // copies the old one byte for byte, and a byte copy out of a block already retyped to `i32`
    // has no meaning in the value model. `Global::grow` reallocates in place instead.
    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        unsafe { Global.grow(ptr, old_layout, new_layout) }
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // The value under test: grows past its capacity, is indexed, and is dropped — the same
    // shapes 21_vec pins for the default allocator.
    let mut custom: Vec<i32, Arena> = Vec::new_in(Arena);
    for value in 1..=5 {
        custom.push(value * value);
    }
    print_usize(custom.len());
    print_i32(custom[3]);

    let mut total = 0;
    for value in &custom {
        total += *value;
    }
    print_i32(total);
}
