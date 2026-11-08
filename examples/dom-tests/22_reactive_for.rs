//! A `for` whose list re-renders: the search island's last blocker.
//!
//! Before this a `for` filled its hole with a plain value, so the list of rows was built once and a
//! later change to the collection was never seen. A loop whose ITERATED EXPRESSION reads a signal
//! is now filled through `view_abi::effect` instead, which hands `_$insert` an accessor rather than
//! an array; the runtime subscribes to the accessor, so writing the signal renders the list again
//! with the rows the collection now holds. That is upstream's `insert(parent, () => rows, marker)`.
//!
//! The iterated expression is the whole test, and the three functions below are why:
//!
//! * `filtered` iterates something derived from `query`, so its hole is reactive. It is also keyed,
//!   which is the combination the search island needs: the list re-runs, and a row whose key
//!   survives the re-run keeps the DOM node it already had.
//! * `unkeyed` is the same reactive list without a key clause, so the accessor form does not depend
//!   on keying.
//! * `fixed` is a loop over a static slice in a view that DECLARES a signal. Its hole is written
//!   once, because the rule is per loop and not per view. A `$(..)` inside the row is a reactive
//!   hole of the row's own template and stays one: re-running the list for it would rebuild every
//!   row and fire that hole as well.
//!
//! No `.family`. Corpus family 13 is the unkeyed static loop, and its reference module is solid's
//! `<For>`, a keyed reconciling component; comparing a reactive Rust loop against it would measure
//! the construct difference rather than this lowering.
//!
//! The emission is pinned by `22_reactive_for.js.expected`. What the emission MEANS is pinned by
//! `scripts/reactive-for-check.mjs`, which captures the accessor `_$insert` was handed, writes the
//! signal, and calls the accessor again: the dom suite's trace records that an insert happened but
//! never re-runs anything, so a re-render is invisible to it. Same limitation, and same answer, as
//! the keyed rows in `20_keyed_for.rs` and the event accessors in `21_event_accessors.rs`. The three
//! views are exported so that check can build one at a time and tell their inserts apart.

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

/// The collection the loops draw from. A `static` rather than an argument, so the accessor the
/// runtime keeps re-running has nothing to borrow.
static POSTS: [Post; 4] = [
    Post { id: 1, title: "alpha" },
    Post { id: 2, title: "beta" },
    Post { id: 3, title: "alto" },
    Post { id: 4, title: "gamma" },
];

/// The search island's shape: a signal the input writes, and a keyed list that re-renders from it.
#[unsafe(no_mangle)]
pub fn filtered() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            signal query = "";
            <input @input=$(|e| query.set(e.target_value())) />
            <ul>
                for post in POSTS.iter().filter(|post| post.title.starts_with(query.get()))
                    key (post.id)
                {
                    <li>(post.title)</li>
                }
            </ul>
        </div>
    }
}

/// The same reactive list with no key clause: the accessor is what makes it re-render, and keying
/// is what makes the nodes survive the re-render. The two are separate.
#[unsafe(no_mangle)]
pub fn unkeyed() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            signal query = "";
            <ul>
                for post in POSTS.iter().filter(|post| post.title.starts_with(query.get())) {
                    <li>(post.title)</li>
                }
            </ul>
        </div>
    }
}

/// A loop that names no signal, in a view that has one. Written once, exactly as before.
#[unsafe(no_mangle)]
pub fn fixed() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            signal count = 0u32;
            <p>$(count.get())</p>
            <ul>
                for post in POSTS.iter() {
                    <li>(post.title)</li>
                }
            </ul>
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = filtered();
    let _ = unkeyed();
    let _ = fixed();
}
