//! Family 02 -- text interpolation.
//!
//! Where a text node ends and a hole begins. Topcoat quotes every text node, so
//! the whitespace ambiguity JSX has does not exist here -- which is precisely
//! the delta this family records.

use view_dom_macro::dom_view;

fn trailing() -> view_abi::Node {
    dom_view! {
        <span>"Hello "</span>
    }
}

fn leading() -> view_abi::Node {
    dom_view! {
        <span>" John"</span>
    }
}

fn trailing_expr(name: &str) -> view_abi::Node {
    dom_view! {
        <span>"Hello " (name)</span>
    }
}

fn leading_expr(greeting: &str) -> view_abi::Node {
    dom_view! {
        <span>(greeting) " John"</span>
    }
}

fn multi_expr(greeting: &str, name: &str) -> view_abi::Node {
    dom_view! {
        <span>(greeting) " " (name)</span>
    }
}

fn multi_expr_together(greeting: &str, name: &str) -> view_abi::Node {
    dom_view! {
        <span>" " (greeting) (name) " "</span>
    }
}

fn escape() -> view_abi::Node {
    dom_view! {
        <span>"\u{a0}<Hi>\u{a0}"</span>
    }
}

fn injection() -> view_abi::Node {
    dom_view! {
        <span>"Hi" ("<script>alert();</script>")</span>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = trailing;
    let _ = leading;
    let _ = trailing_expr;
    let _ = leading_expr;
    let _ = multi_expr;
    let _ = multi_expr_together;
    let _ = escape;
    let _ = injection;
}
