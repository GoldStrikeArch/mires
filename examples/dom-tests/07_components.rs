//! Corpus family 07: component invocations with eager props.
//!
//! The cases are `contract/fixtures/corpus/07-components/view.rs`, and the fns here are its client
//! halves copied verbatim: a client component is a plain Rust fn taking ONE argument, the props
//! struct `#[component(client)]` declares for it, destructured on the first line. That destructure
//! is the equivalence this family is built on -- a component call is a `createComponent` call whose
//! props object is destructured at the callee's boundary -- and it is why the emitted thunk reaches
//! the runtime as `_$createComponent(() => badge$fn({ label: "Active" }))`: the props object is the
//! Rust struct, and a struct argument is a JavaScript object keyed by field name.
//!
//! `_$createComponent` is what makes the props object's position matter beyond bookkeeping. It
//! swaps `sharedConfig.context` for a nested one before calling what it is given and restores the
//! parent afterwards, so a component's hydration keys nest inside its caller's (CONTRACT-DOM 9.5).
//! Nothing here observes that -- these templates are client-only, so no key is claimed at all --
//! and `07b_nested.rs` is the fixture that does.
//!
//! Two cases of the corpus family have no counterpart here, both because the macro still refuses
//! them rather than because the backend cannot emit them:
//!
//!   * `with_children`, because child nodes of a component are rejected by
//!     `view-dom/src/lower/component.rs`. The server writes a hydration key where a part renders
//!     and an eager child renders inside the callee, so the two halves disagree on the order the
//!     caller's slots are spent; `keys-nested.json`'s `getterChildren` rows pin the order the
//!     eager form has to produce.
//!   * a component at the top level of a view, because the key context is opened around the
//!     insert, so `with_children`'s outer `card(...)` has no template to hang from either way.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// ------------------------------------------------------------------ components

/// One static prop. The body is one template with one write-once hole. The prop borrows, so the
/// struct names `'__tc_props` and the use of it elides.
struct BadgeProps<'__tc_props> {
    label: &'__tc_props str,
}

fn badge(props: BadgeProps<'_>) -> view_abi::Node {
    let BadgeProps { label } = props;
    dom_view_client_only! {
        <span class="badge">(label)</span>
    }
}

/// A reactive prop as a signal handle. `Sig<f64>` does not borrow, so this struct takes no lifetime
/// parameter. The prop is passed once; the tracking read is the `$()` in this body, not anything at
/// the call site.
struct LiveCountProps {
    count: view_abi::Sig<f64>,
}

fn live_count(props: LiveCountProps) -> view_abi::Node {
    let LiveCountProps { count } = props;
    dom_view_client_only! {
        <p class="count">"count " $(count.get())</p>
    }
}

/// A reactive prop as a closure: the same shape one indirection further out. `impl Fn() -> f64` is
/// not legal in field position, so the argument-position `impl Trait` is desugared the way Rust
/// already defines it, into a generic parameter both items carry.
struct ReadoutProps<R: Fn() -> f64> {
    read: R,
}

fn readout<R: Fn() -> f64>(props: ReadoutProps<R>) -> view_abi::Node {
    let ReadoutProps { read } = props;
    dom_view_client_only! {
        <p class="readout">$(read())</p>
    }
}

// ------------------------------------------------------------------ call sites

/// Static props only. Nothing here is reactive, so nothing needs a closure.
fn static_props() -> view_abi::Node {
    dom_view_client_only! {
        <div id="main">
            badge(label: "Active")
        </div>
    }
}

/// A prop read out of the surrounding scope. Still eager: `label` is passed once, which is what
/// Rust argument passing already means.
fn dynamic_prop(label: &str) -> view_abi::Node {
    dom_view_client_only! {
        <div id="main">
            badge(label: label)
        </div>
    }
}

/// The reactive path. The signal is declared in the caller and its handle is the prop; `live_count`
/// reads it in its own reactive hole.
fn reactive_prop() -> view_abi::Node {
    dom_view_client_only! {
        signal count = 0.0f64;

        <div class="panel">
            live_count(count: count)
            <button @click=$(|_e: &view_abi::Event| count.set(count.get() + 1.0))>"+1"</button>
        </div>
    }
}

/// The closure path, for a prop that is a derivation rather than a signal. `R` is inferred from the
/// props struct the emitted thunk builds.
fn closure_prop() -> view_abi::Node {
    dom_view_client_only! {
        signal count = 0.0f64;

        <div class="panel">
            readout(read: (move || count.get() * 2.0))
        </div>
    }
}

/// Two calls in one body, which is where the order props are evaluated in is recorded: left to
/// right, each one before its own component body runs. Two separate props structs are built, and
/// two separate thunks close over them.
fn sibling_components() -> view_abi::Node {
    dom_view_client_only! {
        <div class="row">
            badge(label: "One")
            badge(label: "Two")
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = static_props();
    let _ = dynamic_prop("Active");
    let _ = reactive_prop();
    let _ = closure_prop();
    let _ = sibling_components();
}
