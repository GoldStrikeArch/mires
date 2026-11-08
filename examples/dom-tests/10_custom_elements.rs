//! Corpus family 10: custom elements.
//!
//! The cases are `contract/fixtures/corpus/10-custom-elements/view.rs`. Dashed element and
//! attribute names are ordinary Topcoat syntax, so the markup carries over with no escape hatch.
//!
//! # What this family measures
//!
//! `is_import_node`. A template holding any element with a dashed tag name is instantiated with
//! `document.importNode` rather than `cloneNode`, so a custom element upgrades on insertion, and
//! the cloner is emitted as `_$template(html, true, false, false)`. The flag is per template, not
//! per element: `with_children`'s `<my-element>` root carries it for the `<header>` inside it too.
//!
//! # Where the two sides part, and why neither closes
//!
//! Two of the reference's shapes have no Topcoat counterpart, and both are recorded rather than
//! chased.
//!
//! - The reference writes `_el$._$owner = _$getOwner()` on every custom-element root, six times
//!   over this family. No Topcoat program produces it.
//! - The reference compiles an unknown attribute on a custom element to a plain JS PROPERTY write
//!   (`_el$.someAttr = name`), which leaves no runtime call and so no trace record at all, where
//!   Topcoat lowers every `name=(expr)` to `_$setAttribute`.
//!
//! Upstream's `attr:` and `prop:` cases are absent for the reason the family's NOTES.md gives:
//! `attr:my-attr` is written plainly here because the guess for a dashed name agrees anyway, and
//! `prop:someProp` has no Topcoat spelling, so writing it would be a silently wrong pairing rather
//! than a documented one.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// A dashed attribute on a custom element.
fn dashed_attr(name: &str) -> view_abi::Node {
    dom_view_client_only! {
        <my-element some-attr=(name)></my-element>
    }
}

/// A camelCased one, which the reference lowercases into a property and Topcoat writes as given.
fn camel_attr(data: &str) -> view_abi::Node {
    dom_view_client_only! {
        <my-element notProp=(data)></my-element>
    }
}

/// Static children under a custom element: one template, still imported.
fn with_children() -> view_abi::Node {
    dom_view_client_only! {
        <my-element>
            <header slot="head">"Title"</header>
        </my-element>
    }
}

/// No holes at all, so the flag is the only thing that distinguishes this cloner.
fn static_only() -> view_abi::Node {
    dom_view_client_only! {
        <my-widget data-widget-id="profile"></my-widget>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = dashed_attr("profile");
    let _ = camel_attr("value");
    let _ = with_children();
    let _ = static_only();
}
