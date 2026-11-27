//! The host's own data structures, as Rust types.
//!
//! A program compiled by `rustc_codegen_js` runs on a JavaScript engine, and that engine already
//! ships hash tables: `Map` and the string interning behind every object key. This crate hands
//! those to Rust as [`collections::HashMap`] and [`collections::HashSet`].
//!
//! # Why not `hashbrown`
//!
//! `std`'s `HashMap` is `hashbrown`, and `hashbrown` cannot be compiled by this backend. Its group
//! scan reads sixteen control bytes as one wide integer and punnes the result back to a bitmask,
//! and its one allocation holds control bytes and table entries at two different element
//! granularities. Both are byte-level reinterpretations of a buffer, and in this value model a
//! heap block is a JavaScript array of one element type (`CONTRACT.md`, "Allocation"): a block is
//! retyped once, and reading the same bytes at a second width is a run time refusal rather than a
//! wrong number. So the table has to come from somewhere else, and the host has one.
//!
//! # What a map is at run time
//!
//! A [`collections::HashMap`] **is** a host `Map`. There is no handle table, no side allocation and
//! no `Drop`: the value the backend keeps in the Rust local is the `Map` object itself, and the
//! engine's garbage collector frees it when nothing names it any more. See `CONTRACT.md`,
//! "Host collections".

#![no_std]
#![feature(local_waker)]

extern crate alloc;

/// One transparent word representing an opaque host-managed JavaScript value.
///
/// Every handle wrapper in this crate is `#[repr(transparent)]` over `HostHandle`. The backend
/// peels transparent layers down to the inner `u32` (never read by Rust code) while keeping each
/// outer type's `PinnedBoundary::Host` identity.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub(crate) struct HostHandle(
    #[allow(dead_code, reason = "opaque host-side handle populated by JS lowering")] u32,
);

#[cfg(test)]
impl HostHandle {
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

pub mod collections;
pub mod web;

pub use js_extern_macro::{js_bindings, js_export, js_extern};

mod executor;
mod host;
mod host_value;
mod promise;

pub use executor::*;
pub use host::*;
pub use host_value::*;
pub use promise::*;
pub use web::*;
