//! The `text` marker: what a hole's value renders as, and which values need formatting.
//!
//! A hole is handed the JavaScript representation of whatever value it was given, and for most
//! values that is not the text a reader wants to see. `view_abi::text` is wrapped around a
//! non-literal hole value and the backend lowers it against the monomorphized type:
//!
//! * a type whose representation is already a string is the identity;
//! * `bool` and every integer and float go through `String(x)`, which agrees with Rust's own
//!   `Display` for all of them; a `char` is a code point NUMBER in the value model, so it goes
//!   through `String.fromCodePoint` instead, or `'J'` would render as `74`;
//! * a reference is read through, which is what retires writing `(*title)` in a view to get at a
//!   `&&str` (see `13_topcoat_for.rs`, whose rows iterate by reference);
//! * anything else is formatted by its own `Display`, streamed into the host string builder by
//!   `view_abi::display_to_str`. That is ordinary Rust running at run time, not a conversion the
//!   emitter knows: `Version`'s `{major}.{minor}` below is produced by its own impl.
//!
//! There is no corpus family for this. The reference compiler has no counterpart -- JSX
//! interpolates JavaScript values, whose `toString` is the language's own -- so this fixture's
//! goldens are the expectation.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// A type with a hand-written `Display`, so the general path has something to format that no
/// conversion could guess.
struct Version {
    major: u32,
    minor: u32,
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "v{}.{}", self.major, self.minor)
    }
}

/// The identity cases: a `&str` is already a string.
fn borrowed_str(name: &str) -> view_abi::Node {
    dom_view_client_only! { <span>(view_abi::text(name))</span> }
}

/// A double reference, which is the `(*title)` case written without the workaround.
fn double_ref(title: &&str) -> view_abi::Node {
    dom_view_client_only! { <span>(view_abi::text(title))</span> }
}

/// The primitive conversions, one hole each so the golden pins them separately.
fn primitives(n: i32, ratio: f64, flag: bool, initial: char) -> view_abi::Node {
    dom_view_client_only! {
        <ul>
            <li>(view_abi::text(n))</li>
            <li>(view_abi::text(ratio))</li>
            <li>(view_abi::text(flag))</li>
            <li>(view_abi::text(initial))</li>
        </ul>
    }
}

/// An owned `String`. Not an identity case: `String` is a `Vec` of bytes in the value model, not a
/// JavaScript string, so it takes the `Display` path like any other type.
fn owned_string(owned: String) -> view_abi::Node {
    dom_view_client_only! { <span>(view_abi::text(owned))</span> }
}

/// The general path: a struct with its own `Display`.
fn display_impl(version: Version) -> view_abi::Node {
    dom_view_client_only! { <span>(view_abi::text(version))</span> }
}

/// Rows iterated by reference, which is what `13_topcoat_for` needed `(*item)` for.
fn rows(titles: &[&'static str]) -> view_abi::Node {
    dom_view_client_only! {
        <ul>
            for title in titles {
                <li>(view_abi::text(title))</li>
            }
        </ul>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = borrowed_str("John");
    let title = "Hello";
    let _ = double_ref(&title);
    let _ = primitives(-7, 1.5, true, 'J');
    let _ = owned_string(String::from("owned"));
    let _ = display_impl(Version { major: 1, minor: 4 });
    let titles: [&'static str; 2] = ["A", "B"];
    let _ = rows(&titles);
}
