//! Family 06 -- child holes and where their anchors land.
//!
//! `(expr)` in child position is a `Child` hole. The emitter writes an `<!>`
//! anchor unless the hole is the element's sole child, and every anchor shifts
//! the walk paths of the siblings after it -- which is the whole point of this
//! family.

use view_dom_macro::dom_view;

fn only(children: &str) -> view_abi::Node {
    dom_view! {
        <div>(children)</div>
    }
}

fn after_text(children: &str) -> view_abi::Node {
    dom_view! {
        <div>"Hello " (children)</div>
    }
}

fn before_element(children: &str) -> view_abi::Node {
    dom_view! {
        <div>
            (children)
            <span></span>
        </div>
    }
}

fn between(children: &str) -> view_abi::Node {
    dom_view! {
        <div>
            <span></span>
            (children)
            <span></span>
        </div>
    }
}

fn adjacent(first: &str, second: &str) -> view_abi::Node {
    dom_view! {
        <div>(first) (second)</div>
    }
}

fn dynamic_call(children: impl Fn() -> &'static str) -> view_abi::Node {
    dom_view! {
        <div>$(children())</div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = only;
    let _ = after_text;
    let _ = before_element;
    let _ = between;
    let _ = adjacent;
    let _ = dynamic_call;
}
