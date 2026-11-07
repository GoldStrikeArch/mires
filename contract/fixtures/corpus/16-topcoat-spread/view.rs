//! Family 16 -- attribute spreads, `<div (attrs)>`.
//!
//! LOWERS: `lower/attributes.rs:23` constructs `HoleKind::Spread`. The family is
//! nonetheless EXCLUDED from the L2 comparison, because the spike has no type to
//! write a spread's value with -- see NOTES.md and
//! `contract/fixtures/l2-status.json`.
//!
//! A spread is the one attribute form that can invalidate the template itself:
//! the emitter cannot know at compile time which names the value carries, so a
//! name that would have been baked into the template HTML may have to move out
//! of it. The order cases below exist to pin which side wins.

use topcoat::view::{attributes, Attributes};
use view_dom_macro::dom_view;

fn spread_only(attrs: Attributes) -> view_abi::Node {
    dom_view! {
        <div (attrs)></div>
    }
}

fn spread_before(attrs: Attributes) -> view_abi::Node {
    dom_view! {
        <div (attrs) id="main"></div>
    }
}

fn spread_after(attrs: Attributes) -> view_abi::Node {
    dom_view! {
        <div id="main" (attrs)></div>
    }
}

fn spread_between(attrs: Attributes) -> view_abi::Node {
    dom_view! {
        <div class="base" (attrs) id="main"></div>
    }
}

fn spread_call() -> view_abi::Node {
    dom_view! {
        <div (attributes! { data-state="ready" })></div>
    }
}

fn spread_with_children(attrs: Attributes) -> view_abi::Node {
    dom_view! {
        <div (attrs)>
            <span>"Save"</span>
        </div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = spread_only;
    let _ = spread_before;
    let _ = spread_after;
    let _ = spread_between;
    let _ = spread_call;
    let _ = spread_with_children;
}
