//! Family 07b -- components nested inside components, which is where hydration
//! keys are decided.
//!
//! PARTLY LOWERABLE as of wave 1, for the same reasons as family 07: a component
//! invocation lowers to a `HoleKind::Component` hole
//! (`view-dom/src/lower/component.rs`), and what is missing is the backend's
//! `_$createComponent` emission plus the SSR `Formatter::enter_component` that
//! has to agree with it. `children_component` below is additionally rejected at
//! the call site, because child nodes of a component are not lowered yet
//! (`lower/component.rs:24-32`).
//!
//! Family 07 is about props. This one is about the key namespace. Every component
//! call opens a child hydration context whose id is the parent slot the call
//! spent, so the keys inside a component depend on where the call sits, and the
//! keys after it depend on how many slots it spent. The oracle for all of it is
//! `contract/fixtures/keys-nested.json`, group `corpus-07b`, whose case names are
//! the fn names below. The scheme itself is CONTRACT-DOM.md points 9.5 to 9.8.
//!
//! Two shapes look the same in the source and are not the same in the key
//! namespace:
//!
//!   `nested_in_body`      a component called inside another component's body.
//!                         The inner call happens while the outer context is
//!                         current, so the inner keys nest under the outer.
//!
//!   `children_component`  a component passed as another component's child
//!                         content. Child content is an eager `Node` field of the
//!                         props struct, so the inner call happens BEFORE the
//!                         outer boundary and its keys are siblings of the outer
//!                         component's, not children. Solid puts them under the
//!                         outer component instead, because its children are a
//!                         getter read inside the body. That divergence is
//!                         permanent and quantified in NOTES.md.
//!
//! PROPS ARE A STRUCT, not a positional argument list: a client component takes
//! one argument, the struct both halves declare (PascalCase name + `Props`), and
//! destructures it on the first line. Family 07's view.rs documents the rule and
//! its lifetime handling; each pair below is what `#[component(client)]` emits
//! for that signature. The key layout is unaffected by it -- a props struct is
//! built at the call site, inside the caller's context, before the thunk the hole
//! carries is ever called.

use view_dom_macro::dom_view;

// ------------------------------------------------------------------ components

/// The leaf. One template root, so one key.
struct BadgeProps<'__tc_props> {
    label: &'__tc_props str,
}

fn badge(props: BadgeProps<'_>) -> view_abi::Node {
    let BadgeProps { label } = props;
    dom_view! {
        <span class="badge">(label)</span>
    }
}

/// Takes child content as an eager `Node` field.
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

/// Calls another component from its own body, which is the pure nesting case:
/// one root key for the section, then a child context for `badge`.
struct LabelledCardProps<'__tc_props> {
    title: &'__tc_props str,
}

fn labelled_card(props: LabelledCardProps<'_>) -> view_abi::Node {
    let LabelledCardProps { title } = props;
    dom_view! {
        <section class="card">
            <h2>(title)</h2>
            badge(label: "Active")
        </section>
    }
}

/// Two component calls in one body, so the child context's counter has to
/// advance between them.
struct DoubleCardProps<'__tc_props> {
    title: &'__tc_props str,
}

fn double_card(props: DoubleCardProps<'_>) -> view_abi::Node {
    let DoubleCardProps { title } = props;
    dom_view! {
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
    dom_view! {
        <section class="outer">
            <h2>(title)</h2>
            labelled_card(title: "Inner")
        </section>
    }
}

// ------------------------------------------------------------------ call sites

/// `keys-nested.json` case `07b-component-in-body`: keys "00" then "010".
fn nested_in_body() -> view_abi::Node {
    dom_view! {
        labelled_card(title: "Profile")
    }
}

/// `keys-nested.json` case `07b-children-component-eager`: keys "00" then "10".
/// The badge is a field of the props struct, so its boundary comes first and the
/// two components are siblings in the root context. REJECTED TODAY by
/// `lower/component.rs:24-32`.
fn children_component() -> view_abi::Node {
    dom_view! {
        card(
            title: "Profile",
            badge(label: "Active")
        )
    }
}

/// `keys-nested.json` case `07b-depth-three`: keys "00", "010", "0110".
fn depth_three() -> view_abi::Node {
    dom_view! {
        outer_card(title: "Profile")
    }
}

/// `keys-nested.json` case `07b-siblings-in-body`: keys "00", "010", "020".
fn siblings_in_body() -> view_abi::Node {
    dom_view! {
        double_card(title: "Profile")
    }
}

/// `keys-nested.json` case `07b-nested-in-element`: keys "0", "10", "110". The
/// element takes root slot 0, so the component opens at slot 1.
fn nested_in_element() -> view_abi::Node {
    dom_view! {
        <div id="main">
            labelled_card(title: "Profile")
        </div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = badge;
    let _ = card;
    let _ = labelled_card;
    let _ = double_card;
    let _ = outer_card;
    let _ = nested_in_body;
    let _ = children_component;
    let _ = depth_three;
    let _ = siblings_in_body;
    let _ = nested_in_element;
}
