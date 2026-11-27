//! The host `Map`, and the marker calls that reach it.
//!
//! Every operation here is a marker function: an ordinary Rust `fn` with an unreachable body,
//! recognized by its `rcgjs.map.*` `link_section` and lowered directly to JavaScript `Map`
//! operations by `jsc-link/src/lower/js/host.rs`.
//!
//! # Storage model
//!
//! This module is value-agnostic; the collection types above it pick what each host `Map` entry
//! stores:
//!
//! * [`super::HashMap`] stores a non-transparent `MapEntry<V>` wrapper around `Option<V>` so
//!   `&MapEntry<V>` and `&mut MapEntry<V>` always stay indirect even when `Option<V>` uses
//!   null-niche lowering, and moving old values out on `insert` and `remove` is safe
//!   `Option::replace` / `Option::take`.
//! * [`super::HashSet`] stores `bool` values directly to avoid wrapper allocations.

use crate::HostHandle;

/// A host `Map`, as the value model sees it: the `Map` object itself.
///
/// One transparent machine word ([`HostHandle`]): a zero-sized value is folded away by MIR, and a
/// value copied out of an ordinary one-field struct is rebuilt field by field, so a map passed by
/// value through either shape would lose its host object identity.
///
/// `Copy` because the handle is a reference to the host object rather than an owner of it: a
/// collection type above this one decides what owning means. Nothing frees a `Map`; the engine's
/// collector does when nothing names it.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub(crate) struct RawMap {
    /// Never read by Rust code; see the type docs.
    #[allow(dead_code, reason = "opaque host Map handle lowered by backend")]
    handle: HostHandle,
}

/// `new Map()`.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.new")]
#[inline(never)]
pub(crate) fn map_new() -> RawMap {
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `map.size`.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.len")]
#[inline(never)]
pub(crate) fn map_len(map: RawMap) -> usize {
    let _ = map;
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `map.clear()`.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.clear")]
#[inline(never)]
pub(crate) fn map_clear(map: RawMap) {
    let _ = map;
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `map.has(key)`.
///
/// The key is taken **by reference** and every marker below takes it the same way, because that is
/// the one shape that covers both key classes: a `&str` already IS the host string, and a `&i32`
/// is a `{ buf, off }` slot the backend reads through. Taking a key by value would leave the
/// string class with nothing to pass.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.has")]
#[inline(never)]
pub(crate) fn map_has<Q: ?Sized>(map: RawMap, key: &Q) -> bool {
    let _ = (map, key);
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `map.set(key, value)`.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.set")]
#[inline(never)]
pub(crate) fn map_set<Q: ?Sized, V>(map: RawMap, key: &Q, value: V) {
    let _ = (map, key, value);
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `map.delete(key)`, answering whether the key was present.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.del")]
#[inline(never)]
pub(crate) fn map_del<Q: ?Sized>(map: RawMap, key: &Q) -> bool {
    let _ = (map, key);
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `map.get(key)`, as a shared reference to the stored value, or `None` if absent.
///
/// # Safety
///
/// * Every entry stored in `map` under a key of `Q`'s class must have been stored as `V`.
/// * No mutable reference to the returned entry may exist, and `map` must not mutate or remove
///   that entry, for the lifetime `'a`.
#[allow(
    unsafe_code,
    reason = "declares unsafe host JS Map FFI marker whose contract is enforced by safe wrappers"
)]
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.try_get")]
#[inline(never)]
pub(crate) unsafe fn map_try_get<'a, Q: ?Sized, V>(map: RawMap, key: &Q) -> Option<&'a V> {
    let _ = (map, key);
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `map.get(key)`, as a mutable reference to the stored value, or `None` if absent.
///
/// # Safety
///
/// * Every entry stored in `map` under a key of `Q`'s class must have been stored as `V`.
/// * The caller must hold exclusive access to `map` for the lifetime `'a` so no other reference to
///   the returned entry exists, and the entry must not be removed or overwritten while the
///   returned reference is live.
#[allow(
    unsafe_code,
    reason = "declares unsafe host JS Map FFI marker whose contract is enforced by safe wrappers"
)]
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.try_getmut")]
#[inline(never)]
pub(crate) unsafe fn map_try_get_mut<'a, Q: ?Sized, V>(map: RawMap, key: &Q) -> Option<&'a mut V> {
    let _ = (map, key);
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `[...map.keys()]`, the keys as they are right now.
///
/// A snapshot: the array is the host's and the map is free to change afterwards without the array
/// noticing. `S` is [`crate::collections::MapKey::Snapshot`], which is the key class for a key that
/// is a value and `&'static str` for the string class, so the array's elements are exactly what the
/// host stored.
///
/// # Safety
///
/// * Every key stored in `map` must have the host snapshot representation `S`.
/// * `'a` must be bounded by the caller's borrow of the map owning `map` (and any lifetime carried
///   by `S`).
#[allow(
    unsafe_code,
    reason = "declares unsafe host JS Map FFI marker whose contract is enforced by safe wrappers"
)]
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.keys")]
#[inline(never)]
pub(crate) unsafe fn map_keys<'a, S>(map: RawMap) -> &'a [S] {
    let _ = map;
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `[...map.values()]`, the stored entry references in the exact insertion order of [`map_keys`].
///
/// # Safety
///
/// * Every value stored in `map` must have been stored as `V`.
/// * No mutable reference to any entry in `map` may exist, and `map` must not be mutated for the
///   lifetime `'a`.
#[allow(
    unsafe_code,
    reason = "declares unsafe host JS Map FFI marker whose contract is enforced by safe wrappers"
)]
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.values")]
#[inline(never)]
pub(crate) unsafe fn map_values<'a, V>(map: RawMap) -> &'a [&'a V] {
    let _ = map;
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}
