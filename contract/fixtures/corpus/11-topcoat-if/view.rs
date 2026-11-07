//! Family 11 -- `if` / `else if` / `else` with markup bodies.
//!
//! LOWERS: `lower/node.rs:21-24` makes a markup-bodied `if` a reactive child
//! hole whose closure chooses a branch. Only the attribute-position form is
//! still refused (`lower/attributes.rs:37`).
//!
//! The key was claimed BEFORE the error while this was unsupported, deliberately:
//! it kept the site numbering identical between the DOM emitter and the SSR one
//! even while the DOM side refused the node, so numbering did not shift when the
//! feature landed. Status per family: `contract/fixtures/l2-status.json`.

use view_dom_macro::dom_view;

fn if_else(signed_in: bool) -> view_abi::Node {
    dom_view! {
        <div>
            if signed_in {
                <a href="/account">"Account"</a>
            } else {
                <a href="/login">"Sign in"</a>
            }
        </div>
    }
}

fn if_only(signed_in: bool) -> view_abi::Node {
    dom_view! {
        <div>
            if signed_in {
                <a href="/account">"Account"</a>
            }
        </div>
    }
}

fn if_else_if(a: bool, b: bool) -> view_abi::Node {
    dom_view! {
        <div>
            if a {
                <span>"a"</span>
            } else if b {
                <span>"b"</span>
            } else {
                <span>"fallback"</span>
            }
        </div>
    }
}

/// `if` in an ATTRIBUTE list emits attributes rather than nodes. JSX has no
/// equivalent -- a JSX conditional can only choose one attribute's value.
fn attr_if(current: bool) -> view_abi::Node {
    dom_view! {
        <a
            href="/posts"
            if current {
                aria-current="page"
                class="active"
            }
        >
            "Posts"
        </a>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = if_else;
    let _ = if_only;
    let _ = if_else_if;
    let _ = attr_if;
}
