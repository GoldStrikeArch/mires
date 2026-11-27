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

pub mod class_handle;
pub mod collections;
pub mod web;

pub use js_extern_macro::{js_bindings, js_export, js_extern};

mod executor;
mod host;
mod host_value;
mod map_entries;
mod promise;

pub use executor::*;
pub use host::*;
pub use host_value::*;
pub use map_entries::{JsBTreeMapEntries, JsMapEntries};
pub use promise::*;
pub use web::*;

/// Internal runtime support items referenced by `#[js_export]` macro expansions.
#[doc(hidden)]
pub mod __export {
    pub use alloc::string::String;

    pub use crate::class_handle;
    pub use crate::host::JsValue;
    pub use crate::map_entries::{JsBTreeMapEntries, JsMapEntries};
    pub use crate::promise::TypedPromise;
}

#[cfg(test)]
mod export_crate_override_tests {
    use alloc::collections::BTreeMap;
    use alloc::string::String;

    use crate::js_export;

    #[js_export(crate = crate)]
    async fn ping_from_mires_js(label: &str) -> usize {
        label.len()
    }

    #[js_export(crate = crate)]
    fn count_keys(map: BTreeMap<String, u32>) -> usize {
        map.len()
    }

    #[js_export(crate = crate)]
    struct BoxedAcc {
        total: u32,
    }

    #[js_export(crate = crate)]
    impl BoxedAcc {
        pub fn new(total: u32) -> Self {
            Self { total }
        }

        pub fn add(&mut self, delta: u32) -> u32 {
            self.total += delta;
            self.total
        }

        pub fn finish(self) -> u32 {
            self.total
        }
    }

    #[test]
    fn crate_override_exports_work_natively() {
        let mut map = BTreeMap::new();
        map.insert(String::from("a"), 1);
        assert_eq!(count_keys(map), 1);

        let mut acc = BoxedAcc::new(3);
        assert_eq!(acc.add(4), 7);
        assert_eq!(acc.finish(), 7);

        let _async_fn = ping_from_mires_js("ok");
    }
}
