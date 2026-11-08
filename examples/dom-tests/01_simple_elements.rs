//! Corpus family 01: fully static markup, no holes and no reactivity.
//!
//! The cases are `contract/fixtures/corpus/01-simple-elements/view.rs`, written against
//! `dom_view_client_only!` because the reference trace this is compared with was recorded from the
//! non-hydratable preset.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn nested() -> view_abi::Node {
    dom_view_client_only! {
        <div id="main">
            <h1>"Welcome"</h1>
            <label for="entry">"Edit:"</label>
            <input id="entry" type="text">
        </div>
    }
}

fn siblings() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            <span>
                <a></a>
            </span>
            <span></span>
        </div>
    }
}

fn raw_text() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            <style>"div { color: red; }"</style>
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = nested();
    let _ = siblings();
    let _ = raw_text();
}
