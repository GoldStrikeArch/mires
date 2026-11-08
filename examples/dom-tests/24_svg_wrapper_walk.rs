//! A hole inside a wrapped SVG root: the walk has to start past the synthetic `<svg>`.
//!
//! Corpus family 09 has no case like this, and it is the one that would fail silently. A template
//! rooted at an SVG-only element is emitted as `<svg>` plus the element plus `</svg>`, and the
//! runtime hands back `t.content.firstChild.firstChild`, so the node every walk starts from is the
//! wrapper's first child. `Template::tree` descends past the wrapper to match
//! (`contract/CONTRACT-DOM.md` 2.2).
//!
//! # Why this shape and not another
//!
//! Most walks give the same answer at either depth, so most shapes prove nothing. This one does
//! not. `root_group`'s `<g>` has a dynamic first child with a sibling after it, so the walk reaches
//! a `<!>` anchor and the insert is anchored:
//!
//! ```js
//! $t0 = _0.firstChild;
//! _$insert(_0, label, $t0);
//! ```
//!
//! Rooted at the wrapper instead, the same walk reaches the `<g>` element rather than the anchor,
//! the hole reads as an element's sole child, and the emitted call becomes `_$insert($t0, label)`.
//! That inserts into the anchor comment at run time, with nothing anywhere reporting it. The golden
//! below is what tells the two apart.
//!
//! `root_text` is the second half of the pair: an SVG-only root whose hole is its element's only
//! child, so the walk is empty and the insert takes no marker. It pins that the wrapper does not
//! turn a sole child into an anchored one either.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// A dynamic first child under a wrapped root, followed by a sibling: the anchored case.
fn root_group(label: &str) -> view_abi::Node {
    dom_view_client_only! {
        <g>
            (label)
            <text x="0" y="0">"px"</text>
        </g>
    }
}

/// The same wrapping, with the hole as its element's only child: the unanchored case.
fn root_text(label: &str) -> view_abi::Node {
    dom_view_client_only! {
        <text x="0" y="0">(label)</text>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = root_group("40");
    let _ = root_text("41");
}
