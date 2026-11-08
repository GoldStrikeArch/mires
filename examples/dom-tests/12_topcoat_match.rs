//! Corpus family 12: `match` with markup arms.
//!
//! A markup-bodied `match` is a reactive child hole whose closure selects an arm, with a guard
//! left on the arm it was written on. The cases are
//! `contract/fixtures/corpus/12-topcoat-match/view.rs`, without the attribute-position form and
//! without the multi-node arm, both of which the macro still refuses.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

enum Status {
    Draft,
    Published { title: &'static str },
    Archived,
}

fn simple_match(status: Status, show_archived: bool) -> view_abi::Node {
    dom_view_client_only! {
        <div>
            match status {
                Status::Draft => <span>"Draft"</span>,
                Status::Published { title } => <a href="/posts">(title)</a>,
                Status::Archived if show_archived => <span>"Archived"</span>,
                _ => "",
            }
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = simple_match(Status::Draft, true);
    let _ = simple_match(Status::Published { title: "Hello" }, false);
    let _ = simple_match(Status::Archived, true);
}
