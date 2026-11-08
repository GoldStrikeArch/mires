//! Corpus family 09: SVG.
//!
//! The cases are `contract/fixtures/corpus/09-svg/view.rs`, ported case for case. Element and
//! attribute names carry over unchanged: Topcoat takes a tag name verbatim and its attribute names
//! already allow `-`, so `linearGradient`, `gradientTransform` and `stroke-width` need no escaping.
//!
//! # What this family measures
//!
//! The namespace consequence, which is the emitter's to infer rather than the author's to write.
//! Three of the four templates are rooted at `<svg>`, which is an ordinary HTML element to the
//! fragment parser, so they take no flag at all. `root_rect` is rooted at an SVG-only element, so
//! `Template::finish` wraps it in a literal `<svg>` and sets `is_svg`, and the cloner is emitted as
//! `_$template(html, false, true, false)`. The two halves are one decision: `isSVG` makes the
//! runtime unwrap two levels, so a flag with no wrapper, or a wrapper with no flag, silently yields
//! the wrong root node (`contract/CONTRACT-DOM.md` 2.2). `root_rect`'s wrapper is also why the
//! reference keeps a trailing `</svg>` where every other template in the corpus leaves its closing
//! tags open, which the family's own delta rule accounts for.
//!
//! # Where the two sides part
//!
//! `dynamic_svg`. JSX has one syntax for a dynamic attribute and the plugin wraps every one in an
//! effect, so the reference shares one `_$effect` and a `_p$` record across the three attributes of
//! the `<rect>`; Topcoat's `name=(expr)` is written once, so ours is three bare `_$setAttribute`
//! calls and no effect. That is the standing `write-once-vs-always-reactive` divergence, and the
//! reference's effect throws before reaching its own writes because its values are healed free
//! bindings.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// A static subtree rooted at `<svg>`: no flag, no wrapper.
fn static_svg() -> view_abi::Node {
    dom_view_client_only! {
        <svg width="400" height="180">
            <rect stroke-width="2" x="50" y="20" width="150" height="150"></rect>
        </svg>
    }
}

/// Dynamic attributes on an SVG element. They are holes, so they leave the template HTML.
fn dynamic_svg(width: &str, x: &str, y: &str) -> view_abi::Node {
    dom_view_client_only! {
        <svg width="400" height="180">
            <rect stroke-width=(width) x=(x) y=(y) width="150" height="150"></rect>
        </svg>
    }
}

/// The camelCased SVG names, which survive verbatim in both the tag and the attribute.
fn camel_cased() -> view_abi::Node {
    dom_view_client_only! {
        <svg>
            <linearGradient gradientTransform="rotate(25)">
                <stop offset="0%"></stop>
            </linearGradient>
        </svg>
    }
}

/// An SVG-only element as the template root: the wrapped and flagged case.
fn root_rect() -> view_abi::Node {
    dom_view_client_only! {
        <rect x="50" y="20" width="150" height="150"></rect>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = static_svg();
    let _ = dynamic_svg("2", "50", "20");
    let _ = camel_cased();
    let _ = root_rect();
}
