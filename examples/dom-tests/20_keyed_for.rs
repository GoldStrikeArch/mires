//! A keyed `for`: `for pat in expr key (expr) { .. }`, and what the key buys.
//!
//! A view's `for` is an eager Rust loop, so every row is built on every render. What a key buys is
//! that a row whose key was seen before contributes the NODE it contributed last time, so
//! reordering a list moves the existing nodes instead of replacing them and whatever DOM state they
//! carry survives. The key is any `Display` value and is compared by the text it renders as, which
//! is the same conversion `view_abi::text` does.
//!
//! `reorder` is the case this fixture exists for, and it is asserted in the trace rather than
//! described: the same three keys are rendered twice in a different order, and the second render's
//! `insert` names the SAME three nodes as the first, in the new order. An unkeyed loop would name
//! three freshly cloned nodes.
//!
//! The cost, and the reason this is a delta against the runtime's own `_$mapArray` rather than an
//! implementation of it: the row for a cached key is still BUILT and then discarded, because the
//! Rust loop cannot be asked not to run. Keyed tracking that skips building needs the collection
//! handed over as a JavaScript value plus a row function the runtime calls, and a Rust iterator is
//! neither. `view_abi::list` says the same at more length. The cache is also never pruned, so a key
//! that stops appearing keeps its node alive; both are documented on `__rt.keyed_row`.
//!
//! No corpus family: family 13 is the unkeyed loop, and its reference module is solid's `<For>`,
//! which is a keyed reconciling COMPONENT rather than a loop. Comparing against it would measure
//! the construct difference, not this lowering.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Post {
    id: u32,
    title: &'static str,
}

/// The keyed loop. The key is a field of the item, evaluated inside the loop.
fn keyed(posts: &[Post]) -> view_abi::Node {
    dom_view_client_only! {
        <ul>
            for post in posts key (post.id) {
                <li>(post.title)</li>
            }
        </ul>
    }
}

/// The same shape unkeyed, so the golden shows both lowerings side by side: `push` against
/// `push_keyed`.
fn unkeyed(posts: &[Post]) -> view_abi::Node {
    dom_view_client_only! {
        <ul>
            for post in posts {
                <li>(post.title)</li>
            }
        </ul>
    }
}

/// A key that is not a number, to pin that a key is converted the way a text hole value is.
fn keyed_by_str(posts: &[Post]) -> view_abi::Node {
    dom_view_client_only! {
        <ul>
            for post in posts key (post.title) {
                <li>(post.id)</li>
            }
        </ul>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let first = [
        Post { id: 1, title: "A" },
        Post { id: 2, title: "B" },
        Post { id: 3, title: "C" },
    ];
    let _ = keyed(&first);

    // The same keys, reordered. Every row is rebuilt by the loop and every one of them is
    // discarded in favour of the node the key already had, so the insert names the first render's
    // nodes in the new order.
    let reordered = [
        Post { id: 3, title: "C" },
        Post { id: 1, title: "A" },
        Post { id: 2, title: "B" },
    ];
    let _ = keyed(&reordered);

    // A key never seen before contributes its own new node, beside two cached ones.
    let grown = [
        Post { id: 2, title: "B" },
        Post { id: 4, title: "D" },
        Post { id: 1, title: "A" },
    ];
    let _ = keyed(&grown);

    let _ = unkeyed(&first);
    let _ = keyed_by_str(&first);
}
