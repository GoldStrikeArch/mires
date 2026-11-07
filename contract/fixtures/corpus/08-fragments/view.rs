//! Family 08 -- fragments.
//!
//! Topcoat has no fragment syntax because it does not need one: a view body
//! with several top-level nodes IS a fragment. view-dom lowers it to one
//! `Template` per root and returns a tuple, so the return type states the arity.
//!
//! The one thing that does NOT carry over: a top-level node that is not an
//! element. `DomWriter::template` rejects bare text and bare `(expr)` outside an
//! element ("the dom emitter only lowers nodes inside an element"), so the
//! upstream fragments whose first or last child is an expression have no
//! Topcoat equivalent at the top level.

use view_dom_macro::dom_view;

fn multi_static() -> (view_abi::Node, view_abi::Node) {
    dom_view! {
        <div>"First"</div>
        <div>"Last"</div>
    }
}

fn multi_expression(inserted: &str) -> (view_abi::Node, view_abi::Node, view_abi::Node) {
    dom_view! {
        <div>"First"</div>
        <div>(inserted)</div>
        <div>"Last"</div>
    }
}

fn first_dynamic(inserted: impl Fn() -> &'static str) -> (view_abi::Node, view_abi::Node) {
    dom_view! {
        <div>$(inserted())</div>
        <div></div>
    }
}

fn last_static(inserted: &str) -> (view_abi::Node, view_abi::Node) {
    dom_view! {
        <div></div>
        <div>(inserted)</div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = multi_static;
    let _ = multi_expression;
    let _ = first_dynamic;
    let _ = last_static;
}
