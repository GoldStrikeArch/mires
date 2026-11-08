//! Corpus family 14: `let pat = expr;` inside a view body.
//!
//! A `let` is compile time plumbing: it introduces a Rust binding scoped to the rest of the body
//! and contributes no DOM, so the emitted template is the one the inlined expression would have
//! produced. The cases are `contract/fixtures/corpus/14-topcoat-local/view.rs`.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Post {
    title: &'static str,
}

impl Post {
    fn url(&self) -> &str {
        "/hello"
    }
}

fn node_local(post: Post) -> view_abi::Node {
    dom_view_client_only! {
        <article>
            let title = post.title.trim();

            <h1>(title)</h1>
            <a href=(post.url())>"Read"</a>
        </article>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = node_local(Post { title: " Hello " });
}
