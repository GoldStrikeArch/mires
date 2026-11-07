//! Family 12 -- `match` with markup arms.
//!
//! LOWERS: `lower/node.rs:29-32` makes a `match` with markup arms a reactive
//! child hole. Only the attribute-position form is still refused
//! (`lower/attributes.rs:45`).
//!
//! An arm body is ONE view node, and several sibling nodes need a block. Both
//! halves landed: a block groups siblings without opening a scope of its own
//! (`lower/node.rs:21`), so `multi_node_arm` is expressible.
//! Status per family: `contract/fixtures/l2-status.json`.

use view_dom_macro::dom_view;

enum Status {
    Draft,
    Published { title: &'static str },
    Archived,
}

enum State {
    Open,
    Closed,
}

fn simple_match(status: Status, show_archived: bool) -> view_abi::Node {
    dom_view! {
        <div>
            match status {
                Status::Draft => <span>"Draft"</span>,
                Status::Published { title } => <a href="/posts">(title)</a>,
                Status::Archived if show_archived => <span>"Archived"</span>,
                _ => "",
            }
        </div>
    }
}

fn multi_node_arm(user: Option<&str>) -> view_abi::Node {
    dom_view! {
        <div>
            match user {
                Some(name) => {
                    <h1>(name)</h1>
                    <p>"Signed in"</p>
                },
                None => <a href="/login">"Sign in"</a>,
            }
        </div>
    }
}

/// `match` in an attribute list: each arm emits attributes, not nodes.
fn attr_match(state: State) -> view_abi::Node {
    dom_view! {
        <article
            match state {
                State::Open => class="open",
                State::Closed => aria-disabled="true",
            }
        ></article>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = simple_match;
    let _ = multi_node_arm;
    let _ = attr_match;
}
