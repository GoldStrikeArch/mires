//! Family 03 -- attribute expressions.
//!
//! Static attributes bake into the template HTML; `name=(expr)` becomes an
//! `Attribute` hole. Topcoat has no classList/style-object attribute, so those
//! upstream cases are dropped rather than approximated.

use view_dom_macro::dom_view;

struct Post {
    url: &'static str,
    title: &'static str,
    slug: &'static str,
}

fn static_attrs() -> view_abi::Node {
    dom_view! {
        <div id="main" class="base"></div>
    }
}

fn dynamic_value(id: &str) -> view_abi::Node {
    dom_view! {
        <h1 id=(id)>"Welcome"</h1>
    }
}

fn dynamic_member(post: &Post) -> view_abi::Node {
    dom_view! {
        <a href=(post.url) title=(post.title)></a>
    }
}

fn boolean_attr() -> view_abi::Node {
    dom_view! {
        <input type="text" disabled="">
    }
}

fn mixed(post: &Post) -> view_abi::Node {
    dom_view! {
        <div id="main">
            <a href="/" data-slug=(post.slug)>"Welcome"</a>
        </div>
    }
}

fn property(value: &str) -> view_abi::Node {
    dom_view! {
        <input value=(value)>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = static_attrs;
    let _ = dynamic_value;
    let _ = dynamic_member;
    let _ = boolean_attr;
    let _ = mixed;
    let _ = property;
}
