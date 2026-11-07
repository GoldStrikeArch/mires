//! Family 14 -- `let pat = expr;` bindings inside a view body.
//!
//! LOWERS: `lower/local.rs:6`. Only the attribute-list form is still refused
//! (`lower/attributes.rs:48`). Unlike the control-flow nodes, `let` claims NO
//! key -- it is a binding, not a reactive scope.
//! Status per family: `contract/fixtures/l2-status.json`.
//!
//! A `let` is pure compile-time plumbing: it introduces a Rust binding scoped to
//! the rest of the body. Nothing about it should reach the emitted JavaScript,
//! so the expected lowering is that the template is identical to one written
//! with the expression inlined.

use view_dom_macro::dom_view;

struct Post {
    title: &'static str,
    slug: &'static str,
}

impl Post {
    fn url(&self) -> &str {
        "/hello"
    }
}

fn node_local(post: Post) -> view_abi::Node {
    dom_view! {
        <article>
            let title = post.title.trim();

            <h1>(title)</h1>
            <a href=(post.url())>"Read"</a>
        </article>
    }
}

/// A `let` in an attribute list. The binding is in scope for every attribute
/// that follows it.
fn attr_local(post: Post) -> view_abi::Node {
    dom_view! {
        <a
            let href = post.url();
            href=(href)
            data-slug=(post.slug)
        >
            (post.title)
        </a>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = node_local;
    let _ = attr_local;
}
