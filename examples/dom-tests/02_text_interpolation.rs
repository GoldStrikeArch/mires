//! Corpus family 02: where a text node ends and a hole begins.
//!
//! The cases are `contract/fixtures/corpus/02-text-interpolation/view.rs`. Topcoat quotes every
//! text node, so the whitespace ambiguity JSX has does not exist here, which is one of the deltas
//! this family records.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn trailing() -> view_abi::Node {
    dom_view_client_only! { <span>"Hello "</span> }
}

fn leading() -> view_abi::Node {
    dom_view_client_only! { <span>" John"</span> }
}

fn trailing_expr(name: &str) -> view_abi::Node {
    dom_view_client_only! { <span>"Hello " (name)</span> }
}

fn leading_expr(greeting: &str) -> view_abi::Node {
    dom_view_client_only! { <span>(greeting) " John"</span> }
}

fn multi_expr(greeting: &str, name: &str) -> view_abi::Node {
    dom_view_client_only! { <span>(greeting) " " (name)</span> }
}

fn multi_expr_together(greeting: &str, name: &str) -> view_abi::Node {
    dom_view_client_only! { <span>" " (greeting) (name) " "</span> }
}

fn escape() -> view_abi::Node {
    dom_view_client_only! { <span>"\u{a0}<Hi>\u{a0}"</span> }
}

fn injection() -> view_abi::Node {
    dom_view_client_only! { <span>"Hi" ("<script>alert();</script>")</span> }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = trailing();
    let _ = leading();
    let _ = trailing_expr("world");
    let _ = leading_expr("Hello");
    let _ = multi_expr("Hello", "John");
    let _ = multi_expr_together("Hello", "John");
    let _ = escape();
    let _ = injection();
}
