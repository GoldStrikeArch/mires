//! Corpus family 05: a conditional used as a value rather than as control flow.
//!
//! A Rust `if` inside `(...)` or `$(...)` is a hole whose value happens to be chosen by a test.
//! The markup-bodied form is family 11. The cases are
//! `contract/fixtures/corpus/05-conditional/view.rs`.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn static_test(simple: bool, good: &'static str, bad: &'static str) -> view_abi::Node {
    dom_view_client_only! {
        <div>(if simple { good } else { bad })</div>
    }
}

fn dynamic_test(dynamic: bool, good: &'static str, bad: &'static str) -> view_abi::Node {
    dom_view_client_only! {
        <div>$(if dynamic { good } else { bad })</div>
    }
}

fn logical_and(dynamic: bool, good: &'static str) -> view_abi::Node {
    dom_view_client_only! {
        <div>$(if dynamic { good } else { "" })</div>
    }
}

fn chain(a: bool, b: bool) -> view_abi::Node {
    dom_view_client_only! {
        <div>$(if a { "a" } else if b { "b" } else { "fallback" })</div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = static_test(true, "good", "bad");
    let _ = dynamic_test(false, "good", "bad");
    let _ = logical_and(true, "good");
    let _ = chain(false, true);
}
