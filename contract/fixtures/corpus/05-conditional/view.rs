//! Family 05 -- conditionals expressed as a value, not as control flow.
//!
//! A Rust `if` used as an EXPRESSION inside `(...)` or `$(...)` is a hole whose
//! value happens to be chosen conditionally. That is the form this family
//! covers; markup-bodied `if` is family 11.

use view_dom_macro::dom_view;

fn static_test(simple: bool, good: &'static str, bad: &'static str) -> view_abi::Node {
    dom_view! {
        <div>(if simple { good } else { bad })</div>
    }
}

fn dynamic_test(dynamic: bool, good: &'static str, bad: &'static str) -> view_abi::Node {
    dom_view! {
        <div>$(if dynamic { good } else { bad })</div>
    }
}

fn logical_and(dynamic: bool, good: &'static str) -> view_abi::Node {
    dom_view! {
        <div>$(if dynamic { good } else { "" })</div>
    }
}

fn chain(a: bool, b: bool) -> view_abi::Node {
    dom_view! {
        <div>$(if a { "a" } else if b { "b" } else { "fallback" })</div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = static_test;
    let _ = dynamic_test;
    let _ = logical_and;
    let _ = chain;
}
