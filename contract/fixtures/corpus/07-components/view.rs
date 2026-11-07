//! Family 07 -- component invocations with eager props.
//!
//! PARTLY LOWERABLE as of wave 1. `view-dom/src/lower/component.rs` lowers a
//! component invocation to a `HoleKind::Component` hole
//! (`view-dom/src/dom_writer.rs:215-230`), so the old "not yet supported by the
//! dom emitter" error at `lower/node.rs` is gone and `HoleKind::Component` is
//! constructed for the first time. What is still missing is the BACKEND: the
//! `_$createComponent` emission behind `backend/src/abi.rs`'s
//! `HoleKind::Component => zombie(...)`. Two call-site forms are also still
//! rejected, each with a spanned error:
//!
//!   * child nodes of a component (`lower/component.rs:24-32`), so
//!     `with_children` below does not compile yet;
//!   * a `$(...)` runtime expression as a prop value
//!     (`lower/component.rs:49-56`), which is why a reactive prop is passed as a
//!     value that carries its own reactivity rather than as an expression.
//!
//! THE FORM THE EMITTER SUPPORTS
//! -----------------------------
//! A client component is a plain Rust fn taking ONE argument: the props struct
//! both halves of the component declare, named after the component in PascalCase
//! with `Props` appended (`topcoat_view_grammar::component::props_ident`). The
//! function destructures it on the first line. Props are eager: each field is an
//! ordinary Rust value, evaluated exactly once where the component is called, in
//! source order, before the call. A prop that must stay reactive is passed as
//! something carrying its own reactivity -- a `view_abi::Sig<T>` or a closure --
//! and the tracking read happens inside the callee's own `$()` hole.
//!
//! Props are named at the call site rather than ordered, and the struct is what
//! makes the call independent of the order the callee declared them in: the
//! emitter cannot see that order. A missing prop is "missing field" and an
//! unknown one is "no field named", both at the call site.
//!
//! The struct borrows for `'__tc_props` if any prop borrows, and takes no
//! lifetime at all otherwise (`view-dom/src/client_component.rs:196-260`).
//!
//! The fns below are the CLIENT half. `#[component(client)]` generates both
//! halves from one source: the server `#[component]` (async, returns
//! `topcoat::Result`, parameters passed through POSITIONALLY, child content typed
//! `View`) and this one under `cfg(topcoat_client)`. The corpus measures the dom
//! emitter, so what is written here is the half the dom emitter sees. Each pair
//! below is byte-for-byte what `#[component(client)]` emits for that signature.
//!
//! The CALL SITES at the bottom are not components. They stand for an island
//! entry point, whose client half takes its parameters positionally
//! (`view-dom/src/island.rs:163-174`), so they are plain fns.
//!
//! Nesting a component inside another component is family 07b, because the
//! interesting part of that is hydration keys rather than props.

use view_dom_macro::dom_view;

// ------------------------------------------------------------------ components

/// One static prop. The body is one template with one write-once hole. The prop
/// borrows, so the struct names `'__tc_props` and the use of it elides.
struct BadgeProps<'__tc_props> {
    label: &'__tc_props str,
}

fn badge(props: BadgeProps<'_>) -> view_abi::Node {
    let BadgeProps { label } = props;
    dom_view! {
        <span class="badge">(label)</span>
    }
}

/// Child content as an eager `Node`, which is an ordinary field of the props
/// struct. `(child)` is a write-once hole whose value happens to be a node the
/// caller built. `child` is the field trailing view nodes desugar to.
struct CardProps<'__tc_props> {
    title: &'__tc_props str,
    child: view_abi::Node,
}

fn card(props: CardProps<'_>) -> view_abi::Node {
    let CardProps { title, child } = props;
    dom_view! {
        <section class="card">
            <h2>(title)</h2>
            (child)
        </section>
    }
}

/// A reactive prop as a signal handle. `Sig<f64>` does not borrow, so this
/// struct takes no lifetime parameter. The prop is passed once; the tracking
/// read is the `$()` in this body, not anything at the call site.
struct LiveCountProps {
    count: view_abi::Sig<f64>,
}

fn live_count(props: LiveCountProps) -> view_abi::Node {
    let LiveCountProps { count } = props;
    dom_view! {
        <p class="count">"count " $(count.get())</p>
    }
}

/// A reactive prop as a closure: the same shape one indirection further out. The
/// callee decides when to call it, and the call sits in a `$()` so it is tracked.
///
/// A closure prop is the one signature that is NOT a verbatim copy of the
/// declared parameter list. `impl Fn() -> f64` is not legal in field position, so
/// the argument-position `impl Trait` is desugared the way Rust already defines
/// it: one generic parameter, carried by BOTH items so a use of the struct can
/// name it. Call sites are unaffected because inference fills it in. See
/// NOTES.md; today's `#[component(client)]` copies the type verbatim instead and
/// so emits a struct that does not compile.
struct ReadoutProps<R: Fn() -> f64> {
    read: R,
}

fn readout<R: Fn() -> f64>(props: ReadoutProps<R>) -> view_abi::Node {
    let ReadoutProps { read } = props;
    dom_view! {
        <p class="readout">$(read())</p>
    }
}

// ------------------------------------------------------------------ call sites

/// Static props only. Nothing here is reactive, so nothing needs a closure. The
/// hole the emitter builds is `move || badge(BadgeProps { label: ("Active"), })`.
fn static_props() -> view_abi::Node {
    dom_view! {
        <div id="main">
            badge(label: "Active")
        </div>
    }
}

/// A prop read out of the surrounding scope. Still eager: `label` is passed once,
/// which is what Rust argument passing already means.
fn dynamic_prop(label: &str) -> view_abi::Node {
    dom_view! {
        <div id="main">
            badge(label: label)
        </div>
    }
}

/// Trailing view nodes desugar to the `child:` field, so the `<p>` is built
/// before `card` is called. REJECTED TODAY by `lower/component.rs:24-32`.
fn with_children() -> view_abi::Node {
    dom_view! {
        card(
            title: "Profile",
            <p>"Account details"</p>
        )
    }
}

/// The reactive path. The signal is declared in the caller and its handle is the
/// prop; `live_count` reads it in its own reactive hole. Note that the prop VALUE
/// is a plain expression: `$(...)` as a prop value is rejected, because a prop is
/// evaluated once rather than tracked.
fn reactive_prop() -> view_abi::Node {
    dom_view! {
        signal count = 0.0;

        <div class="panel">
            live_count(count: count)
            <button @click=$(|_e| count.set(count.get() + 1.0))>"+1"</button>
        </div>
    }
}

/// The closure path, for a prop that is a derivation rather than a signal. The
/// emitted hole is `move || readout(ReadoutProps { read: (|| count.get() * 2.0), })`,
/// and `R` is inferred from it.
fn closure_prop() -> view_abi::Node {
    dom_view! {
        signal count = 0.0;

        <div class="panel">
            readout(read: (|| count.get() * 2.0))
        </div>
    }
}

/// Two calls in one body, which is where the corpus records the order props are
/// evaluated in: left to right, each one before its own component body runs. Two
/// separate props structs are built, and two separate thunks close over them.
fn sibling_components() -> view_abi::Node {
    dom_view! {
        <div class="row">
            badge(label: "One")
            badge(label: "Two")
        </div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = badge;
    let _ = card;
    let _ = live_count;
    let _ = readout::<fn() -> f64>;
    let _ = static_props;
    let _ = dynamic_prop;
    let _ = with_children;
    let _ = reactive_prop;
    let _ = closure_prop;
    let _ = sibling_components;
}
