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

extern crate alloc;

/// One transparent word representing an opaque host-managed JavaScript value.
///
/// Every handle wrapper in this crate — and every framework handle annotated with [`js_opaque`] —
/// is `#[repr(transparent)]` over `HostHandle`. The backend peels transparent layers down to the
/// inner `u32` (never constructed or read directly by user Rust code) while keeping each outer
/// type's host-handle identity.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct HostHandle(
    #[allow(dead_code, reason = "opaque host-side handle populated by JS lowering")] u32,
);

impl HostHandle {
    /// Erases this opaque host handle to a [`JsValue`].
    #[must_use]
    pub fn as_value(self) -> JsValue {
        #[cfg(mires)]
        {
            JsValue::erase(self)
        }
        #[cfg(not(mires))]
        {
            JsValue::from_handle(self)
        }
    }

    /// Consumes this opaque host handle and returns it as a [`JsValue`].
    #[must_use]
    pub fn into_value(self) -> JsValue {
        self.as_value()
    }

    /// Wraps a [`JsValue`] as an opaque [`HostHandle`] without performing runtime type checks.
    #[must_use]
    pub fn from_value(value: JsValue) -> Self {
        #[cfg(mires)]
        {
            value.read()
        }
        #[cfg(not(mires))]
        {
            value.into_handle()
        }
    }
}

impl From<HostHandle> for JsValue {
    fn from(handle: HostHandle) -> Self {
        handle.as_value()
    }
}

impl From<JsValue> for HostHandle {
    fn from(value: JsValue) -> Self {
        Self::from_value(value)
    }
}

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

pub use js_extern_macro::{js_bindings, js_export, js_extern, js_opaque};

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

/// Internal runtime support items referenced by `#[js_export]` and `#[js_opaque]` macro expansions.
#[doc(hidden)]
pub mod __export {
    pub use alloc::string::String;

    pub use crate::class_handle;
    pub use crate::host::JsValue;
    pub use crate::map_entries::{JsBTreeMapEntries, JsMapEntries};
    pub use crate::promise::TypedPromise;
    pub use crate::HostHandle;
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

    #[crate::js_opaque(crate = crate)]
    #[derive(Clone, Copy)]
    struct OpaqueNode(crate::HostHandle);

    #[crate::js_opaque(ts = "Signal<T>", crate = crate)]
    struct OpaqueSignal<T>(crate::HostHandle, core::marker::PhantomData<T>);

    #[test]
    fn host_handle_and_js_opaque_round_trip_on_host() {
        let handle = crate::HostHandle::from_raw(77);
        let value = handle.as_value();
        assert_eq!(value.raw(), 77);
        assert_eq!(handle.into_value().raw(), 77);
        assert_eq!(crate::HostHandle::from_value(value).raw(), 77);
        assert_eq!(crate::HostHandle::from(value).raw(), 77);
        assert_eq!(crate::JsValue::from(handle).raw(), 77);

        let node = OpaqueNode::from_value_unchecked(value);
        assert_eq!(OpaqueNode::__MIRES_TS_TYPE, "");
        assert_eq!(node.as_value().raw(), 77);
        assert_eq!(crate::JsValue::from(node).raw(), 77);

        let signal = OpaqueSignal::<u64>::from_value_unchecked(value);
        assert_eq!(OpaqueSignal::<u64>::__MIRES_TS_TYPE, "Signal<T>");
        assert_eq!(signal.as_value().raw(), 77);

        fn check_callback_inference() {
            let _ = crate::callback(|| {});
            let _ = crate::callback(|x: i32| x + 1);
            let _ = crate::callback(|ev: crate::JsValue| {
                let _ = ev;
            });
            let _ = crate::callback(|a: f64, b: f64| a + b);
        }
        let _keep: fn() = check_callback_inference;
    }
}
