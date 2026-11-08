//! Corpus family 16: an attribute spread, `<div (attrs)>`.
//!
//! A spread is a `Spread` hole, and the backend writes it with `_$spread(node, value, false,
//! false)`. What a spread's value *is* has no answer yet: the corpus writes `topcoat::view::
//! Attributes`, which the spike does not build, so this fixture pins the emitted shape and the
//! order a spread takes relative to the attributes baked into the template, not the runtime
//! behaviour. It carries no `.family` for that reason.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Attrs {
    id: &'static str,
}

fn spread_only(attrs: Attrs) -> view_abi::Node {
    dom_view_client_only! {
        <div (attrs)></div>
    }
}

fn spread_after(attrs: Attrs) -> view_abi::Node {
    dom_view_client_only! {
        <div id="main" (attrs)></div>
    }
}

fn spread_with_children(attrs: Attrs) -> view_abi::Node {
    dom_view_client_only! {
        <div (attrs)>
            <span>"Save"</span>
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = spread_only(Attrs { id: "a" });
    let _ = spread_after(Attrs { id: "b" });
    let _ = spread_with_children(Attrs { id: "c" });
}
