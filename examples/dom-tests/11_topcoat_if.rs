//! Corpus family 11: `if` / `else if` / `else` with markup bodies.
//!
//! A markup-bodied `if` is a reactive child hole whose closure chooses a branch, and each branch
//! is a `view_abi::content` erasing whatever it built to one type. A missing `else` renders
//! nothing, which is `content(())` and becomes `null`. The cases are
//! `contract/fixtures/corpus/11-topcoat-if/view.rs`, without the attribute-position form: an `if`
//! in an attribute list is still refused by the macro.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn if_else(signed_in: bool) -> view_abi::Node {
    dom_view_client_only! {
        <div>
            if signed_in {
                <a href="/account">"Account"</a>
            } else {
                <a href="/login">"Sign in"</a>
            }
        </div>
    }
}

fn if_only(signed_in: bool) -> view_abi::Node {
    dom_view_client_only! {
        <div>
            if signed_in {
                <a href="/account">"Account"</a>
            }
        </div>
    }
}

fn if_else_if(a: bool, b: bool) -> view_abi::Node {
    dom_view_client_only! {
        <div>
            if a {
                <span>"a"</span>
            } else if b {
                <span>"b"</span>
            } else {
                <span>"fallback"</span>
            }
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = if_else(true);
    let _ = if_only(false);
    let _ = if_else_if(false, true);
}
