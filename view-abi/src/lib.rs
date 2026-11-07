//! The template ABI between `view!`'s expansion and the codegen backend.
//!
//! A `view!` template compiled for the client is communicated to the backend as a
//! plain `static TemplateData` passed as the first argument of the marker functions
//! below. The backend recognizes the marker `link_section` names, resolves the
//! argument to the static through its constant provenance, decodes it at codegen
//! time with its constant reader, and replaces the call with dom-expressions
//! emission. The static itself carries no attribute: rustc rejects a custom
//! `link_section` on a static holding references, and none is needed: the marker
//! call's argument is the join.
//!
//! Design rule this ABI is built against: every field must be decodable by the
//! backend's existing constant-reader primitives: scalars, `&'static str`,
//! `&'static [T]`, `#[repr(C)]` structs with named fields, and fieldless `repr(u8)`
//! enums with a direct tag. No `Option` of references, no trait objects, no function
//! pointers. Any shape change bumps [`ABI_VERSION`], and the backend refuses a
//! version it does not know by name at the call site.
//!
//! # Markers
//!
//! Every `pub fn` below is a marker: it carries a `link_section` the backend matches
//! on, and its body is unreachable. A marker is a Rust-ABI generic function on
//! purpose, so the backend sees a monomorphized instance per call site and can name
//! the argument types it needs to emit.

#![no_std]

/// The version the backend checks before trusting the rest of the payload.
///
/// Version 2 added [`Hole::effect_group`]. Version 3 added the [`cond`],
/// [`handler`], [`list`] and [`push`] markers, and made [`Node`] a transparent
/// one-word handle. Version 4 added the [`component`], [`text`] and [`push_keyed`]
/// markers, and made [`Sig`] a transparent one-word handle taken by value. Version
/// 5 added the four executor markers ([`microtask`], [`on_settled`], [`owner`] and
/// [`with_owner`]) and the [`JsValue`] and [`Owner`] handles. Version 6 added
/// [`TemplateData::is_import_node`] and [`TemplateData::is_svg`], the two
/// template-construction flags `_$template` takes after the HTML.
pub const ABI_VERSION: u32 = 6;

/// The [`Hole::effect_group`] of a hole that no shared effect drives.
pub const NO_EFFECT_GROUP: u32 = u32::MAX;

/// One `view!` template's static skeleton and its dynamic holes.
#[repr(C)]
pub struct TemplateData {
    /// Always [`ABI_VERSION`] at the time the macro expanded.
    pub abi: u32,
    /// The template's HTML, exactly as `_$template` takes it. Must survive an
    /// `innerHTML` round trip node for node, and must carry a `<!>` wherever two
    /// text nodes would otherwise merge.
    pub html: &'static str,
    /// Whether this template is instantiated by claiming server-rendered nodes
    /// rather than by cloning.
    ///
    /// The HTML never carries a `data-hk` attribute: the server writes that
    /// attribute and the client finds the node through the hydration registry. What
    /// the flag changes in the HTML is marker emission, described on
    /// [`Hole::path`]. What it changes in the emitted code is that the backend calls
    /// `_$getNextElement` in place of the cloner.
    pub hydratable: bool,
    /// Whether the cloner instantiates this template with `document.importNode`
    /// instead of `cloneNode`, which is the second argument of `_$template`.
    ///
    /// Set when any element in the template has a dashed tag name. A custom
    /// element upgrades on insertion only if its document owns it, and importing
    /// is what gives the clone that owner.
    pub is_import_node: bool,
    /// Whether the template's root is an SVG-only element, which is the third
    /// argument of `_$template`.
    ///
    /// The flag and the HTML travel together: [`TemplateData::html`] already
    /// carries the literal `<svg>` wrapper the flag's extra unwrap depth expects,
    /// and every [`Hole::path`] is still rooted at the wrapped element rather than
    /// at the wrapper. Setting one without the other silently yields the wrong
    /// root node (`contract/CONTRACT-DOM.md` 2.2).
    pub is_svg: bool,
    /// One entry per hole, in the order the holes appear in the source. The backend
    /// fills holes in exactly this order.
    pub holes: &'static [Hole],
    /// Delegated event names this template needs, deduplicated and sorted.
    pub events: &'static [&'static str],
}

/// One dynamic position inside a template.
#[repr(C)]
pub struct Hole {
    /// The linear walk from the cloned root to the hole's node: one byte per step,
    /// `b'c'` for `firstChild`, `b'n'` for `nextSibling`. Never an index into
    /// `childNodes`.
    ///
    /// For a [`HoleKind::Child`] the walk reaches the node the value is inserted
    /// against, which is one of three things. An empty walk means the value is the
    /// element's only child and is inserted with no marker. In a non-hydratable
    /// template the walk reaches a `<!>` anchor comment. In a hydratable template it
    /// reaches the opening `<!$>` of the `<!$><!/>` pair that brackets the value,
    /// and the backend hands the node after it to `_$getNextMarker`.
    pub path: &'static str,
    pub kind: HoleKind,
    /// The attribute, property or event name; empty where the kind takes none.
    pub name: &'static str,
    /// Whether the value is wrapped in `_$effect` or written once.
    pub reactive: bool,
    /// Which effect drives this hole, or [`NO_EFFECT_GROUP`] for one that no shared
    /// effect drives.
    ///
    /// Reactive holes on the same element share a group, and the backend emits one
    /// effect per group carrying a previous-value record that skips unchanged
    /// writes. A group with a single member needs no record. Child holes and event
    /// holes are never grouped: each child insert owns its computation.
    pub effect_group: u32,
}

/// What a hole holds.
#[repr(u8)]
pub enum HoleKind {
    Child,
    Attribute,
    Property,
    DelegatedEvent,
    Event,
    ClassList,
    Style,
    Spread,
    Component,
}

/// An opaque handle to a template's DOM node, or to a list of them. Only the
/// backend gives these meaning; on the server this type is never constructed.
///
/// Two properties of the declaration are load bearing, and both are about what a
/// value of it survives as on the way through MIR.
///
/// * **One machine word rather than zero-sized**, exactly as on [`Sig`]. MIR folds a
///   zero-sized return to a constant, so every function returning a template root
///   would return `undefined` and every branch of a conditional would contribute
///   nothing.
/// * **`repr(transparent)`**, so the handle *is* the field. An ordinary one-field
///   struct is an object keyed by field name in the backend's value model, and a
///   value copied out of one is rebuilt field by field: a template root passed
///   through a `Node` would come back as `{ handle: undefined }`. Transparent makes
///   both the field read and the rebuild the identity, which is what lets the
///   backend keep the DOM node itself in the handle.
#[repr(transparent)]
pub struct Node {
    /// Never read by Rust code; see the type docs.
    #[allow(dead_code)]
    handle: u32,
}

/// The DOM event an event-handler hole receives.
///
/// Opaque; only the backend gives it meaning. The macro annotates a handler
/// closure's parameter with `&Event` so the closure's type is inferrable when it is
/// passed through [`handler`].
///
/// What a handler can ask it is deliberately small: the value of the element the
/// event came from, and cancelling the browser's default. Anything else belongs to a
/// declared JavaScript interface rather than to a type whose whole purpose is to make
/// a closure's parameter nameable.
pub struct Event(());

impl Event {
    /// The `value` of the element the event came from, as `event.target.value`.
    ///
    /// This is what an `@input` or `@change` handler reads: `<input>`, `<textarea>`
    /// and `<select>` all carry the text the user typed there. An element with no
    /// `value` reads as the empty string.
    ///
    /// The lifetime is `'static` because the string is a JavaScript string the host
    /// owns, not bytes borrowed from the event: a `&str` in the value model IS that
    /// string, so there is nothing here for a shorter lifetime to protect.
    #[must_use]
    pub fn target_value(&self) -> &'static str {
        event_target_value(self)
    }

    /// Cancels the browser's default handling, as `event.preventDefault()`.
    pub fn prevent_default(&self) {
        event_prevent_default(self);
    }
}

/// Backs [`Event::target_value`]. A free function because a `link_section` belongs to
/// an item, and the backend matches markers by section rather than by path.
///
/// The event is taken BY REFERENCE, unlike [`sig_get`]'s handle, and the difference is
/// not an oversight. A handler closure's parameter is already a `&Event`, which is the
/// only way an `Event` is ever reached, and the backend reads a marker's argument
/// itself rather than letting the value model interpret it: what arrives is the DOM
/// event the runtime passed the closure. Taking it by value would ask the value model
/// to copy a value out of a reference it has no representation for.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.etv")]
#[inline(never)]
pub fn event_target_value(event: &Event) -> &'static str {
    let _ = event;
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Backs [`Event::prevent_default`]. Takes the event by reference, as
/// [`event_target_value`] does and for the same reason.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.epd")]
#[inline(never)]
pub fn event_prevent_default(event: &Event) {
    let _ = event;
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// A signal declared by `signal name = init;` in a view body.
///
/// The handle carries the value type so a view body type checks as ordinary Rust.
/// Only the backend gives it meaning. Both properties of the declaration are load
/// bearing, for the same reasons they are on [`Node`].
///
/// * **One machine word rather than zero-sized**: a zero-sized value has no
///   JavaScript representation, so the `[get, set]` pair the backend stores in it
///   would be dropped on the floor.
/// * **`repr(transparent)`**, so the handle *is* the field. In the backend's value
///   model an ordinary struct is an object keyed by field name, and a value copied
///   out of one is rebuilt field by field: a signal passed by value -- into a
///   component's props struct, or captured by a `move` closure -- would come back as
///   `{ handle: undefined, marker: undefined }`, and reading it would call
///   `undefined`. Transparent makes both the field read and the rebuild the identity,
///   which is what lets the backend keep the `[get, set]` pair in the handle.
#[repr(transparent)]
pub struct Sig<T> {
    /// Never read by Rust code. The field exists so the type is one machine word,
    /// giving the backend's `[get, set]` pair a place to live.
    #[allow(dead_code)]
    handle: u32,
    marker: core::marker::PhantomData<T>,
}

impl<T> Sig<T> {
    /// Reads the signal, subscribing the effect that is running.
    ///
    /// Takes `self` by value rather than by reference, for the reason [`sig_get`]
    /// gives: a `&Sig<T>` is a reference to a one-word primitive, so a closure that
    /// called this through a reference would capture the address and the value model
    /// would box it. [`Sig`] is `Copy`, so a by-value receiver costs nothing at the
    /// call site and `count.get()` still reads the same.
    #[must_use]
    pub fn get(self) -> T {
        sig_get(self)
    }

    /// Writes the signal, waking every effect that read it.
    pub fn set(self, value: T) {
        sig_set(self, value);
    }
}

impl<T> Clone for Sig<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Sig<T> {}

/// Instantiates a template: clone, walk, return the root.
///
/// Never runs. The backend intercepts the call by its `link_section` and replaces it
/// with `_$template` emission; reaching this body means the crate was compiled by a
/// backend that does not know this ABI.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.template")]
#[inline(never)]
pub fn template(data: &'static TemplateData) -> Node {
    let _ = data;
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Fills hole `index` of an instantiated template with a value, once.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.hole")]
#[inline(never)]
pub fn hole<V>(root: &Node, index: u32, value: V) {
    let _ = (root, index, value);
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Fills hole `index` with an event handler: the backend emits a delegated property
/// or an `addEventListener`, whichever the hole's [`HoleKind`] asks for.
///
/// A handler goes through a marker of its own rather than through [`hole`] because
/// this body can *call* what it is given: the mono collector only walks into a
/// closure whose `Fn::call` some body names, so a handler passed as a plain value
/// would reach the backend with its body never monomorphized and everything it calls
/// missing from the program.
/// The value is discarded, as a DOM listener's is: a handler may be written as an
/// expression with a value, and nothing reads it.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.handler")]
#[inline(never)]
pub fn handler<F, R>(root: &Node, index: u32, f: F)
where
    F: Fn(&Event) -> R,
{
    // The event is built here rather than taken, because only this crate can build
    // one. Nothing ever runs: the backend intercepts every call to this marker.
    let _ = (root, index);
    let _ = f(&Event(()));
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Fills hole `index` reactively: the backend wraps the closure in `_$effect`.
///
/// The marker is a generic Rust-ABI function on purpose: the backend needs the
/// monomorphized closure instance to name its call implementation in the emitted
/// arrow thunk.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.effect")]
#[inline(never)]
pub fn effect<F, V>(root: &Node, index: u32, f: F)
where
    F: Fn() -> V,
{
    // The call is what makes the closure's body reachable: the mono collector only
    // walks into `F::call`, and everything the body calls, because this body names
    // it. The backend intercepts every call to this marker, so nothing ever runs.
    let _ = (root, index);
    let _ = f();
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Fills hole `index` with a client component call: the backend emits
/// `_$createComponent` around it, which is what nests the component's hydration keys
/// inside its caller's.
///
/// The closure takes nothing and returns the component's root. It builds the props
/// struct and calls the component, so the props are evaluated inside the nested
/// hydration context the runtime installs, and the props object the emitted call
/// hands over is the props struct itself.
///
/// A component goes through a marker of its own rather than through [`hole`] for the
/// same reason [`handler`] does: this body *calls* what it is given. The mono
/// collector only walks into a closure whose `Fn::call` some body names, so a
/// component call passed as a plain value would reach the backend with the closure's
/// body, the component function, and everything the component calls all missing from
/// the program.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.component")]
#[inline(never)]
pub fn component<F>(root: &Node, index: u32, f: F)
where
    F: Fn() -> Node,
{
    // Called for the mono collector's sake, as on `effect`. Nothing ever runs.
    let _ = (root, index);
    let _ = f();
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// The text a hole's value renders as.
///
/// Wrapped around a non-literal hole value so that what reaches `_$insert` is a string
/// rather than whatever the value's JavaScript representation happens to be. The
/// backend sees the monomorphized `T` and lowers this one of two ways.
///
/// * A **known primitive** `T` is converted in place, and a `T` that is already a
///   JavaScript string is the identity. This is what retires writing `(*title)` in a
///   view to get at a `&&str`: the deref is the conversion, and the emitter does it.
/// * **Anything else** goes through `T`'s own [`Display`], written into the host
///   string builder by [`display_to_str`], whose instance this body names so the
///   collector walks into it.
///
/// [`Display`]: core::fmt::Display
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.text")]
#[inline(never)]
pub fn text<T: core::fmt::Display>(v: T) -> Node {
    // Naming the general path is what makes it reachable, exactly as `effect` names
    // its closure's call. The backend intercepts every call to this marker, so nothing
    // ever runs.
    let _ = display_to_str(v);
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

// The host string builder the backend's shim keeps, described in `CONTRACT.md` under
// "Allocation". A handle is an index into a table on the JavaScript side, so the whole
// surface is `extern "C"` shaped and none of it allocates in the Rust heap.
//
// `&str` and `char` are not FFI safe in general, and are exactly right here: the other
// side of this boundary is not C, it is the backend's own JavaScript shim, where a `&str`
// arrives as a string and a `char` as a code point number. `CONTRACT.md` is the ABI.
#[allow(improper_ctypes)]
unsafe extern "C" {
    fn sb_new() -> usize;
    fn sb_push(handle: usize, s: &str);
    fn sb_push_char(handle: usize, c: char);
    fn sb_take(handle: usize) -> &'static str;
}

/// A [`core::fmt::Write`] sink that builds its string on the JavaScript side.
///
/// Writing through this spends one host string concatenation per piece rather than a
/// byte push per byte into a heap buffer, and what comes back is already the JavaScript
/// string `_$insert` takes.
struct Sink(usize);

impl core::fmt::Write for Sink {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        unsafe { sb_push(self.0, s) };
        Ok(())
    }

    fn write_char(&mut self, c: char) -> core::fmt::Result {
        unsafe { sb_push_char(self.0, c) };
        Ok(())
    }
}

/// `T`'s [`Display`] output, as a JavaScript string.
///
/// The general half of [`text`], and an ordinary function rather than a marker: the
/// backend emits a call to it for a `T` it has no conversion for, so this body is what
/// runs. The lifetime is a convenient fiction, as it is on `sb_take`: a JavaScript
/// string is a value, so what comes back is owned by nobody and outlives everything.
///
/// [`Display`]: core::fmt::Display
pub fn display_to_str<T: core::fmt::Display>(v: T) -> &'static str {
    use core::fmt::Write as _;

    let handle = unsafe { sb_new() };
    let mut sink = Sink(handle);
    // A `Display` impl may report an error; there is nothing to do about one here and a
    // partial string is still the best answer available.
    let _ = write!(sink, "{v}");
    unsafe { sb_take(handle) }
}

/// Erases a view fragment to a [`Node`], so every branch of a conditional and every
/// row of a loop has one type.
///
/// The value is a template root, a tuple of template roots, `()` for a branch that
/// renders nothing, or any value a text node can hold.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.content")]
#[inline(never)]
pub fn content<V>(value: V) -> Node {
    let _ = value;
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// A conditional in a node position, with its test handed over separately.
///
/// The three closures are the `if`'s test, its `then` branch and its `else` branch;
/// a branch that renders nothing is `content(())`. The backend hoists the test into
/// `_$memo` and returns an accessor that dispatches on it, so a branch is rebuilt
/// only when the test's answer changes rather than whenever anything the branch reads
/// does.
///
/// Handing the test over on its own is what makes that possible: a conditional
/// written as one closure reaches the backend as a single function, and there is no
/// test in it left to hoist.
///
/// An `else if` chain and a `match` with more than two arms nest: the outer `else`
/// closure is one whose body is another `cond`. That mirrors how the reference
/// compiler nests its own memos.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.cond")]
#[inline(never)]
pub fn cond<C, T, E>(test: C, then: T, els: E) -> Node
where
    C: Fn() -> bool,
    T: Fn() -> Node,
    E: Fn() -> Node,
{
    // Called for the mono collector's sake, as on `effect`. Nothing ever runs.
    let _ = (test(), then(), els());
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Starts the list of rows a `for` in a node position renders into.
///
/// The iteration itself stays in Rust: a view's `for` is an ordinary Rust loop over
/// an ordinary Rust iterator, and nothing the backend could synthesize would iterate
/// it. So the loop runs, each row is appended with [`push`], and the list of rows is
/// one value `_$insert` takes.
///
/// This renders a list rather than tracking one: the rows are built where the loop
/// runs, and a later change to the collection is seen only when whatever encloses the
/// `for` runs again. Keyed, per-row tracking is what the runtime's `_$mapArray` does,
/// and it needs the collection handed over as a JavaScript value plus a row function
/// the runtime calls; neither is expressible while the iterator is a Rust one.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.list")]
#[inline(never)]
pub fn list() -> Node {
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Appends one row to a [`list`].
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.push")]
#[inline(never)]
pub fn push(list: &Node, row: Node) {
    let _ = (list, row);
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Appends one row to a [`list`], matched to the row of the same key from the previous
/// render.
///
/// The key is any [`Display`] value, converted to a string the way [`text`] converts a
/// hole value, so a key is compared by the text it renders as. What the backend emits
/// keeps a per-loop cache of the rows it has built: a key seen before contributes the
/// NODE it contributed last time, so reordering a list moves the existing nodes rather
/// than replacing them, and whatever DOM state they carry survives.
///
/// This is a pragmatic reconcile rather than the runtime's `_$mapArray`, and the
/// difference is worth knowing: the Rust loop is eager, so the row for an unchanged key
/// is still BUILT and then discarded in favour of the cached one. Keyed tracking that
/// skips building needs the collection handed over as a JavaScript value plus a row
/// function the runtime calls, which is not expressible while the iterator is a Rust
/// one; [`list`] says the same thing at more length.
///
/// [`Display`]: core::fmt::Display
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.pushkeyed")]
#[inline(never)]
pub fn push_keyed<K: core::fmt::Display>(list: &Node, key: K, row: Node) {
    // `display_to_str` is named for the collector's sake, exactly as in `text`: a key
    // the backend has no conversion for is formatted by this at run time.
    let _ = (list, display_to_str(key), row);
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Declares the signal numbered `ordinal` within its view, seeded with `init`.
///
/// The ordinal comes from the view's key plan, so every emitter numbers the same
/// declaration the same way.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.signal")]
#[inline(never)]
pub fn signal<T>(ordinal: u32, init: T) -> Sig<T> {
    let _ = (ordinal, init);
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Backs [`Sig::get`]. A free function because a `link_section` belongs to an item,
/// and the backend matches markers by section rather than by path.
///
/// The handle is taken BY VALUE, not by reference, and that is load bearing. What the
/// backend keeps in the handle is the runtime's `[get, set]` pair, so the marker has
/// to be handed the pair itself. A `&Sig<T>` is a reference to a one-word primitive,
/// which the value model boxes as soon as anything takes its address, and the marker
/// would then be handed the box rather than the pair. [`Sig`] is `Copy` precisely so
/// that passing it by value costs nothing and every copy names the same signal.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.sget")]
#[inline(never)]
pub fn sig_get<T>(sig: Sig<T>) -> T {
    let _ = sig;
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Backs [`Sig::set`]. Takes the handle by value, for the reason [`sig_get`] gives.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.sset")]
#[inline(never)]
pub fn sig_set<T>(sig: Sig<T>, value: T) {
    let _ = (sig, value);
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

// -------------------------------------------------------------------------------
// The executor
// -------------------------------------------------------------------------------
//
// Four markers, and between them everything a Rust `Future` needs to run inside an
// island. The poll loop itself is Rust, in `view-async`: a loop in JavaScript would
// have to call a monomorphized `Future::poll` through a `&mut F`, and the value
// model hands out neither. What is left on this side is a scheduler, a promise
// bridge, and the two halves of the owner capture.
//
// An island's SETUP stays synchronous whatever runs here (`CONTRACT-DOM` 14.6): the
// hydration window is one synchronous call stack, and a continuation resuming after
// it builds fresh DOM instead of adopting the server's, silently. So these are only
// ever reached from an event handler or an effect.

/// An opaque value belonging to the host: a Promise, an object a declared interface
/// answered, whatever a JavaScript expression produced.
///
/// One machine word and `repr(transparent)`, for the reasons [`Node`] gives: a
/// zero-sized value is folded away by MIR, and a value copied out of an ordinary
/// one-field struct is rebuilt field by field, so a host object passed through one
/// would come back as `{ handle: undefined }`.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsValue {
    /// Never read by Rust code; see the type docs.
    #[allow(dead_code)]
    handle: u32,
}

/// The reactive owner a computation runs under.
///
/// Captured before a future's first poll and re-entered on every resume. Both are
/// required rather than tidy: the runtime's `Owner` and `Listener` are module-level
/// globals restored in a synchronous `finally`, so for an interrupted computation
/// that `finally` fires at the FIRST suspension. A continuation that did not
/// re-enter its owner reads signals without subscribing, registers cleanups that do
/// nothing, and creates effects nobody disposes (`CONTRACT-DOM` 14.3).
///
/// A one-word transparent handle, for the reasons [`Node`] gives.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Owner {
    /// Never read by Rust code; see the type docs.
    #[allow(dead_code)]
    handle: u32,
}

/// Runs `f` in a microtask.
///
/// The one scheduling primitive an executor needs. A microtask rather than a timer
/// because the pinned runtime's own deferral is one (`runHydrationEvents`), and
/// because a task woken during an update pass should resume before the browser
/// paints rather than after it.
///
/// The marker CALLS the closure, as [`effect`] and [`handler`] do and for the same
/// reason: the collector only walks into a closure whose `Fn::call` some body names.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.micro")]
#[inline(never)]
pub fn microtask<F>(f: F)
where
    F: Fn(),
{
    // Called for the collector's sake. The backend intercepts every call to this
    // marker, so nothing ever runs.
    f();
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Calls `f` when `value` settles, with what it settled to and whether it succeeded.
///
/// The bridge from a host Promise to a Rust `Future`. A value that is not a thenable
/// settles immediately and successfully, so awaiting a plain value works and costs
/// one microtask.
///
/// The failure arm is a `false` rather than a rejection on purpose. The pinned
/// runtime has no asynchronous error path at all: `catchError` and `ErrorBoundary`
/// are synchronous `try`/`catch`, so a rejected Promise reaches neither and becomes a
/// host-level unhandled rejection (`CONTRACT-DOM` 14.4). Delivering the failure INTO
/// the poll is what puts it back where Rust can see it, as an `Err`.
///
/// `V` is the caller's claim about what the promise settles to, exactly as it is on
/// [`sig_get`]: the host value IS the Rust value in this model, so a promise of a
/// string awaited as a `&str` needs no conversion. The rejection arrives as a `V`
/// too, because a rejection carries any value at all and only the caller knows which
/// one this interface produces.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.settled")]
#[inline(never)]
pub fn on_settled<V, F>(value: JsValue, f: F)
where
    F: Fn(V, bool),
{
    // Called for the collector's sake, as on [`microtask`]. The settled value is
    // named through `host_value` because this body has no `V` of its own to call
    // with; nothing ever runs, so nothing is ever cast.
    f(host_value(value), true);
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// A host value read as a `V`.
///
/// The identity: in this value model a JavaScript string IS a `&str`, a number IS an
/// `f64`, and an object with the right keys IS a `#[repr(C)]` struct. What the type
/// adds is the caller's claim about which of those this one is, which is the same
/// bargain [`sig_get`] and [`Event::target_value`] already make.
///
/// Private, and reachable only from [`on_settled`]'s body, which the backend
/// replaces: it exists so that body can name `F::call` without having a `V`.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.hostval")]
#[inline(never)]
fn host_value<V>(value: JsValue) -> V {
    let _ = value;
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// The owner the calling code is running under.
///
/// Answered by the runtime's `getOwner`, and meaningful only inside a synchronous
/// computation: after a suspension point it is null (`CONTRACT-DOM` 14.3), which is
/// why an executor captures it before the first poll rather than at a resume.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.owner")]
#[inline(never)]
pub fn owner() -> Owner {
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}

/// Runs `f` under `owner`, restoring whatever was current afterwards.
///
/// The other half of the capture. The runtime's `runWithOwner` saves and restores
/// `Owner` and `Listener` around a synchronous call, which is exactly the shape a
/// resume needs.
///
/// This is the one name the client DOM module owes beyond the 48 the client ABI
/// declares plus `createSignal`: `runWithOwner` is exported by `solid-js` and not
/// re-exported by `solid-js/web`.
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.tc.withowner")]
#[inline(never)]
pub fn with_owner<F>(owner: Owner, f: F)
where
    F: Fn(),
{
    // Called for the collector's sake, as on [`microtask`].
    let _ = owner;
    f();
    unreachable!("view-abi markers are only meaningful to rustc_codegen_mires")
}
