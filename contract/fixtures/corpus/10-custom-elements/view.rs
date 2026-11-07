//! Family 10 -- custom elements.
//!
//! Dashed element and attribute names are ordinary Topcoat syntax. What Topcoat
//! has no spelling for is the `prop:` / `attr:` disambiguation: every
//! `name=(expr)` is lowered as `HoleKind::Attribute`, never `Property`. So the
//! attribute-versus-property decision the reference plugin makes from the name
//! has no Topcoat-side input yet.

use view_dom_macro::dom_view;

fn dashed_attr(name: &str) -> view_abi::Node {
    dom_view! {
        <my-element some-attr=(name)></my-element>
    }
}

fn camel_attr(data: &str) -> view_abi::Node {
    dom_view! {
        <my-element notProp=(data)></my-element>
    }
}

fn with_children() -> view_abi::Node {
    dom_view! {
        <my-element>
            <header slot="head">"Title"</header>
        </my-element>
    }
}

fn static_only() -> view_abi::Node {
    dom_view! {
        <my-widget data-widget-id="profile"></my-widget>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = dashed_attr;
    let _ = camel_attr;
    let _ = with_children;
    let _ = static_only;
}
