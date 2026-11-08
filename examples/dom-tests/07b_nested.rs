//! Corpus family 07b: components nested inside components.
//!
//! Family 07 is about props. This one is about the key namespace: every component call opens a
//! child hydration context whose id is the parent slot the call spent, so the keys inside a
//! component depend on where the call sits and the keys after it depend on how many slots it spent
//! (CONTRACT-DOM 9.5 to 9.8, oracle `contract/fixtures/keys-nested.json` group `corpus-07b`).
//!
//! What this fixture can and cannot show, stated plainly, because the difference decides what it is
//! for:
//!
//! * It PINS the emission. Each nested call is a `_$createComponent` around a thunk, in the order
//!   the contract requires: the props struct is built at the call site inside the caller's context,
//!   and the callee's own template roots are claimed inside the child context the runtime installs.
//!   Routing through `_$createComponent` is what buys the nesting, because that function is the one
//!   that swaps `sharedConfig.context` and restores the parent's already-advanced counter.
//! * It CANNOT pin the keys themselves. The suite runs against `contract/harness/trace.mjs`, whose
//!   `getNextElement` numbers nodes from a flat `hydrationCounter` and which implements no
//!   `sharedConfig` context at all. Nested keys are therefore invisible here whatever the emitter
//!   does. The two places they are checked are the server side (`topcoat-view`'s 102-case
//!   `hydration_keys_nested` test against the oracle) and the parity harness, which runs the real
//!   dom-expressions runtime.
//!
//! Deviations from the corpus family's own call sites, both forced by view-dom rather than chosen:
//!
//! * `children_component` is absent. Child content of a component is rejected
//!   (`view-dom/src/lower/component.rs`), and it is also the family's one permanent divergence from
//!   solid: an eager `Node` prop is built BEFORE the boundary, so its keys are siblings of the outer
//!   component's rather than children of it.
//! * `nested_in_body`, `depth_three` and `siblings_in_body` put a component at the top level of a
//!   view, which is rejected because the key context is opened around the insert and there is no
//!   element to insert into. Each is written here wrapped in a `<div>`, which is the corpus's own
//!   `nested_in_element` shape one level deeper, so the nesting depth each case exists to exercise
//!   is still exercised. The wrapper spends root slot 0, so every key below it shifts by one.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// ------------------------------------------------------------------ components

/// The leaf. One template root, so one key.
struct BadgeProps<'__tc_props> {
    label: &'__tc_props str,
}

fn badge(props: BadgeProps<'_>) -> view_abi::Node {
    let BadgeProps { label } = props;
    dom_view_client_only! {
        <span class="badge">(label)</span>
    }
}

/// Calls another component from its own body, which is the pure nesting case: one root key for the
/// section, then a child context for `badge`.
struct LabelledCardProps<'__tc_props> {
    title: &'__tc_props str,
}

fn labelled_card(props: LabelledCardProps<'_>) -> view_abi::Node {
    let LabelledCardProps { title } = props;
    dom_view_client_only! {
        <section class="card">
            <h2>(title)</h2>
            badge(label: "Active")
        </section>
    }
}

/// Two component calls in one body, so the child context's counter has to advance between them.
struct DoubleCardProps<'__tc_props> {
    title: &'__tc_props str,
}

fn double_card(props: DoubleCardProps<'_>) -> view_abi::Node {
    let DoubleCardProps { title } = props;
    dom_view_client_only! {
        <section class="card">
            <h2>(title)</h2>
            badge(label: "One")
            badge(label: "Two")
        </section>
    }
}

/// Depth three: this body calls `labelled_card`, which calls `badge`.
struct OuterCardProps<'__tc_props> {
    title: &'__tc_props str,
}

fn outer_card(props: OuterCardProps<'_>) -> view_abi::Node {
    let OuterCardProps { title } = props;
    dom_view_client_only! {
        <section class="outer">
            <h2>(title)</h2>
            labelled_card(title: "Inner")
        </section>
    }
}

// ------------------------------------------------------------------ call sites

/// The corpus's `nested_in_element`: the element takes root slot 0, so the component opens at
/// slot 1 and the badge inside it nests one level further.
fn nested_in_element() -> view_abi::Node {
    dom_view_client_only! {
        <div id="main">
            labelled_card(title: "Profile")
        </div>
    }
}

/// The corpus's `depth_three`, wrapped: three boundaries deep.
fn depth_three() -> view_abi::Node {
    dom_view_client_only! {
        <div id="main">
            outer_card(title: "Profile")
        </div>
    }
}

/// The corpus's `siblings_in_body`, wrapped: two sibling boundaries inside one child context.
fn siblings_in_body() -> view_abi::Node {
    dom_view_client_only! {
        <div id="main">
            double_card(title: "Profile")
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = nested_in_element();
    let _ = depth_three();
    let _ = siblings_in_body();
}
