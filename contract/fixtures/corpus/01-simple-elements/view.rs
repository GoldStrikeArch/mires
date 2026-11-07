//! Family 01 -- simple elements.
//!
//! Fully static markup: no holes, no reactivity. Written in the shape of
//! view-dom-macro/tests/expand.rs so it can be dropped in as a test file once
//! the emitter compiles it.

use view_dom_macro::dom_view;

fn nested() -> view_abi::Node {
    dom_view! {
        <div id="main">
            <h1>"Welcome"</h1>
            <label for="entry">"Edit:"</label>
            <input id="entry" type="text">
        </div>
    }
}

fn siblings() -> view_abi::Node {
    dom_view! {
        <div>
            <span>
                <a></a>
            </span>
            <span></span>
        </div>
    }
}

fn raw_text() -> view_abi::Node {
    dom_view! {
        <div>
            <style>"div { color: red; }"</style>
        </div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = nested;
    let _ = siblings;
    let _ = raw_text;
}
