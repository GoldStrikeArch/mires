//! The host `Map`, and the calls that reach it.
//!
//! Two mechanisms, and which one an operation uses is decided by one thing: whether it is generic.
//!
//! * A **monomorphic** operation is an ordinary `extern "C-unwind"` declaration (a host exception
//!   keeps its cleanup edge in the unwind build mode). A foreign call with a C ABI lowers to
//!   `__rt.<symbol>(...)` already (`CONTRACT.md`, "Calls to foreign items"): it costs nothing.
//! * A **generic** operation cannot be a foreign declaration, because rustc has no such thing. It
//!   is a marker function instead: an ordinary Rust `fn` with an unreachable body, recognized by
//!   the `link_section` it carries, exactly as `view-abi`'s markers are (`CONTRACT.md`,
//!   "Templates"). The prefix is `rcgjs.map.` and `backend/src/map.rs` is the lowering.
//!
//! # The value a map stores is an `Option`
//!
//! Nothing here hands the host a bare `V`. A map holds `Option<V>` and the host stores the object
//! that is (`CONTRACT.md`, "Enums"), which buys three things at the cost of one object per entry:
//!
//! * the host never has to know what a Rust value looks like, so the shim stays four lines of `Map`
//!   calls;
//! * `&Option<V>` is always an object rather than sometimes a `{ buf, off }` slot, so the marker
//!   lowering is a plain call with no case analysis on `V`;
//! * moving the old value out on `insert` and `remove` is `Option::replace` and `Option::take`,
//!   which are safe code writing through an ordinary `&mut`, rather than a `ptr::read` that the
//!   caller has to promise is a move.

/// A host `Map`, as the value model sees it: the `Map` object itself.
///
/// One machine word and `repr(transparent)`, for the reasons `view_abi::Node` gives. A zero sized
/// value is folded away by MIR, and a value copied out of an ordinary one field struct is rebuilt
/// field by field, so a map passed by value through either shape would come back as
/// `{ handle: undefined }`.
///
/// `Copy` because the handle is a reference to the host object rather than an owner of it: a
/// collection type above this one decides what owning means. Nothing frees a `Map`; the engine's
/// collector does when nothing names it.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub(crate) struct RawMap {
    /// Never read by Rust code; see the type docs.
    #[allow(dead_code)]
    handle: u32,
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
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.try_get")]
#[inline(never)]
pub(crate) unsafe fn map_try_get<'a, Q: ?Sized, V>(map: RawMap, key: &Q) -> Option<&'a V> {
    let _ = (map, key);
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `map.get(key)`, as a mutable reference to the stored value, or `None` if absent.
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
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.keys")]
#[inline(never)]
pub(crate) fn map_keys<'a, S>(map: RawMap) -> &'a [S] {
    let _ = map;
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}

/// `[...map.values()]`, the stored entry references in the exact insertion order of [`map_keys`].
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.map.values")]
#[inline(never)]
pub(crate) unsafe fn map_values<'a, V>(map: RawMap) -> &'a [&'a V] {
    let _ = map;
    unreachable!("mires-js collection markers are only meaningful to rustc_codegen_js")
}
