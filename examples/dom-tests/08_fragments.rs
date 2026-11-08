//! Corpus family 08 -- fragments.
//!
//! Source: `contract/fixtures/corpus/08-fragments/view.rs`, ported case for case.
//!
//! Topcoat has no fragment syntax because it does not need one: **a view body with several
//! top-level nodes IS a fragment.** `DomWriter::close_scope` collects one template per root and
//! returns a Rust tuple, so the return type states the arity and nothing wraps the roots at run
//! time. That is why this family needed no lowering work at all -- it was a GAP in
//! `G2-CHECKLIST.md` only because nobody had ever compiled it.
//!
//! # The one case that does not carry over
//!
//! A top-level node that is not an element. `DomWriter::template` refuses bare text and a bare
//! `(expr)` outside an element ("the dom emitter only lowers nodes inside an element"), so
//! upstream's `singleExpression`, its trailing `"After"` text and its bare `{inserted}` all get a
//! `<div>` around them in `view.rs`. That is not cosmetic -- it adds a real element to the tree and
//! a real template to the emission -- and it is why this fixture's trace carries a template and a
//! clone the reference does not. `08-fragments/NOTES.md:18-25` records it as a language question
//! rather than a missing feature: "can a view body start with text?" is undecided, not unbuilt.
//!
//! # What the comparison is measuring
//!
//! That a fragment costs NOTHING beyond its roots. The reference emits three templates, six clones
//! and one memo, and no `_$insert` anywhere: a JSX fragment is a plain JavaScript array literal, so
//! nothing is inserted into anything. Ours is the same shape with the extra wrapped root.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Two static roots. The tuple is the fragment.
fn multi_static() -> (view_abi::Node, view_abi::Node) {
    dom_view_client_only! {
        <div>"First"</div>
        <div>"Last"</div>
    }
}

/// Three roots, the middle one a hole. Upstream's middle item is a bare `{inserted}` with no
/// element; this one is wrapped, which is the documented divergence.
fn multi_expression(inserted: &str) -> (view_abi::Node, view_abi::Node, view_abi::Node) {
    dom_view_client_only! {
        <div>"First"</div>
        <div>(inserted)</div>
        <div>"Last"</div>
    }
}

/// A reactive first root, so the memo the reference hoists has something to line up against.
fn first_dynamic(inserted: impl Fn() -> &'static str) -> (view_abi::Node, view_abi::Node) {
    dom_view_client_only! {
        <div>$(inserted())</div>
        <div></div>
    }
}

/// The mirror image: the dynamic root last.
fn last_static(inserted: &str) -> (view_abi::Node, view_abi::Node) {
    dom_view_client_only! {
        <div></div>
        <div>(inserted)</div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = multi_static();
    let _ = multi_expression("middle");
    let _ = first_dynamic(|| "dynamic");
    let _ = last_static("tail");
}
