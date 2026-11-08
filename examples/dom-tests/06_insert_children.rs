//! Corpus family 06: child holes and where their anchors land.
//!
//! The cases are `contract/fixtures/corpus/06-insert-children/view.rs`. `(expr)` in child position
//! is a `Child` hole, and every anchor the emitter writes shifts the walk of the siblings after it.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn only(children: &str) -> view_abi::Node {
    dom_view_client_only! { <div>(children)</div> }
}

fn after_text(children: &str) -> view_abi::Node {
    dom_view_client_only! { <div>"Hello " (children)</div> }
}

fn before_element(children: &str) -> view_abi::Node {
    dom_view_client_only! {
        <div>
            (children)
            <span></span>
        </div>
    }
}

fn between(children: &str) -> view_abi::Node {
    dom_view_client_only! {
        <div>
            <span></span>
            (children)
            <span></span>
        </div>
    }
}

fn adjacent(first: &str, second: &str) -> view_abi::Node {
    dom_view_client_only! { <div>(first) (second)</div> }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = only("a");
    let _ = after_text("b");
    let _ = before_element("c");
    let _ = between("d");
    let _ = adjacent("e", "f");
}
