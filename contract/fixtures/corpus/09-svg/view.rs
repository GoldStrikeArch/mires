//! Family 09 -- SVG.
//!
//! Element names are written exactly as in HTML, so camelCased SVG names such
//! as `linearGradient` and `gradientTransform` survive unchanged. What the
//! emitter must still learn is the namespace consequence: `isSVG` on the
//! template call, `setAttributeNS` for xlink names, and the different unwrap
//! depth an SVG template root needs.

use view_dom_macro::dom_view;

fn static_svg() -> view_abi::Node {
    dom_view! {
        <svg width="400" height="180">
            <rect stroke-width="2" x="50" y="20" width="150" height="150"></rect>
        </svg>
    }
}

fn dynamic_svg(width: &str, x: &str, y: &str) -> view_abi::Node {
    dom_view! {
        <svg width="400" height="180">
            <rect stroke-width=(width) x=(x) y=(y) width="150" height="150"></rect>
        </svg>
    }
}

fn camel_cased() -> view_abi::Node {
    dom_view! {
        <svg>
            <linearGradient gradientTransform="rotate(25)">
                <stop offset="0%"></stop>
            </linearGradient>
        </svg>
    }
}

fn root_rect() -> view_abi::Node {
    dom_view! {
        <rect x="50" y="20" width="150" height="150"></rect>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = static_svg;
    let _ = dynamic_svg;
    let _ = camel_cased;
    let _ = root_rect;
}
