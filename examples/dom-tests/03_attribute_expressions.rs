//! Corpus family 03: attributes whose value is an expression.
//!
//! A static attribute bakes into the template HTML and is not a hole at all; `name=(expr)` is a
//! hole written once. Which sink it reaches is the name's business: `value` on an `input` is a
//! property, everything here else is an attribute. The cases are
//! `contract/fixtures/corpus/03-attribute-expressions/view.rs`.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Post {
    url: &'static str,
    title: &'static str,
    slug: &'static str,
}

fn static_attrs() -> view_abi::Node {
    dom_view_client_only! {
        <div id="main" class="base"></div>
    }
}

fn dynamic_value(id: &str) -> view_abi::Node {
    dom_view_client_only! {
        <h1 id=(id)>"Welcome"</h1>
    }
}

fn dynamic_member(post: &Post) -> view_abi::Node {
    dom_view_client_only! {
        <a href=(post.url) title=(post.title)></a>
    }
}

fn boolean_attr() -> view_abi::Node {
    dom_view_client_only! {
        <input type="text" disabled="">
    }
}

fn mixed(post: &Post) -> view_abi::Node {
    dom_view_client_only! {
        <div id="main">
            <a href="/" data-slug=(post.slug)>"Welcome"</a>
        </div>
    }
}

fn property(value: &str) -> view_abi::Node {
    dom_view_client_only! {
        <input value=(value)>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let post = Post { url: "/hello", title: "Hello", slug: "hello" };
    let _ = static_attrs();
    let _ = dynamic_value("my-h1");
    let _ = dynamic_member(&post);
    let _ = boolean_attr();
    let _ = mixed(&post);
    let _ = property("10");
}
