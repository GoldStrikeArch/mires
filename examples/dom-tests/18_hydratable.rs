//! The hydratable walk: `<!$><!/>` marker pairs, and what a walk does after one.
//!
//! Every other fixture here uses `dom_view_client_only!`, which renders the markup itself. This one
//! uses `dom_view!`, the mode an island's client half is compiled in: the template claims the nodes
//! a server already wrote (`_$getNextElement`) instead of cloning them, and each dynamic child is
//! bracketed by a `<!$><!/>` pair the server rendered its value between.
//!
//! Two things about that are what this fixture pins, and both are invisible in a client-only
//! template:
//!
//! 1. `_$getNextMarker` is handed the node *after* the `<!$>`. Handed the marker itself it scans
//!    past the closing `<!/>`, so the value's own nodes are not collected, and the insert replaces
//!    what the server wrote rather than adopting it.
//! 2. A walk that continues past a pair re-bases on the `<!/>` that `_$getNextMarker` returns. The
//!    template says the two markers are siblings; in the server's DOM the rendered value is between
//!    them, so counting siblings from the `<!$>` lands inside the value.
//!
//! `after_two_holes` is the case that needs both: two dynamic children under one element, and a
//! static text node after them that a raw sibling walk cannot reach.
//!
//! The reference output these are read against is
//! `contract/fixtures/upstream/__dom_hydratable_fixtures__/textInterpolation/output.js`. There is no
//! corpus family for hydratable mode -- the corpus records client-only traces -- so this fixture
//! carries no `.family` and its own golden is the expectation.

#![no_std]
#![no_main]

use view_dom_macro::dom_view;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// One hole, nothing after it: the walk reaches the `<!$>` and the pair closes the template.
fn trailing_hole(name: &str) -> view_abi::Node {
    dom_view! { <span>"Hello " (name)</span> }
}

/// A static text node after the hole, so the walk has to re-base on the closing marker to reach it.
fn hole_then_text(greeting: &str) -> view_abi::Node {
    dom_view! { <span>(greeting) " John"</span> }
}

/// Two holes under one element with static text between and after them: every step after the first
/// pair is wrong without re-basing, and the second pair is reached through the first.
fn after_two_holes(greeting: &str, name: &str) -> view_abi::Node {
    dom_view! { <span>(greeting) " and " (name) " both"</span> }
}

/// A hole under a nested element, with an element after the pair. The re-basing is not only about
/// text runs: the `<p>` is reached from the closing marker too.
fn nested_then_element(name: &str) -> view_abi::Node {
    dom_view! {
        <div>
            <span>"Hello " (name)</span>
            <p>"after"</p>
        </div>
    }
}

/// No hole at all: a hydratable template still claims its root rather than cloning it.
fn no_holes() -> view_abi::Node {
    dom_view! { <span>"Hello John"</span> }
}

/// Twelve dynamic children under one element, which is what puts the tenth hydration key past the
/// point where solid's encoding changes shape (`getContextId` prefixes a letter once the count
/// reaches two digits, so `i0.9` is followed by `i0.a10`).
///
/// Nothing here depends on that. The emitted program contains no key at all: `_$getNextElement` and
/// `_$getNextMarker` ask the runtime, which counts with `sharedConfig` and looks the node up in the
/// registry the server's `data-hk` attributes filled. So the walk this golden pins is the same
/// twelve-deep re-basing chain whatever the keys are spelled like, and a change to the *server's*
/// encoding cannot move it.
fn twelve_holes(v: &str) -> view_abi::Node {
    dom_view! {
        <p>
            (v) "1" (v) "2" (v) "3" (v) "4" (v) "5" (v) "6"
            (v) "7" (v) "8" (v) "9" (v) "10" (v) "11" (v) "12"
        </p>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = trailing_hole("John");
    let _ = hole_then_text("Hello");
    let _ = after_two_holes("Hello", "John");
    let _ = nested_then_element("John");
    let _ = no_holes();
}
