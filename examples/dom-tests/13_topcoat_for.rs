//! Corpus family 13: `for pat in expr` with a markup body.
//!
//! The loop stays Rust. `view_abi::list` starts the list of rows, the loop runs where it is written,
//! and `view_abi::push` appends each row; the list of DOM nodes is one value `_$insert` takes. That
//! is the shape the family's own notes call the structural match: "`.map()` builds an array eagerly
//! and hands it to `insert`, which is what a Rust `for` loop over an iterator does". The recorded
//! reference trace is the *other* JSX form, `<For each={..}>`, which is a keyed reconciling
//! component; the notes record it as idiomatic Solid rather than as the target, so this family's L2
//! comparison differs by that whole construct.
//!
//! Both backings of the same loop are here, because what the backend has to lower is the iterator:
//!
//! * `slice_rows` iterates a `&[T]`, whose `Iter` is pointer arithmetic in `core`;
//! * `vec_rows` iterates a `Vec<T>` by reference, which adds `alloc`'s `RawVec` to the program.
//!
//! `nested_rows` is a loop inside a loop, so an inner list is a row of the outer one.
//!
//! The rows that iterate by reference write `(*item)` rather than `(item)`. A hole is handed the
//! JavaScript representation of whatever value it was given, and the representation of a `&&str` is
//! the slot record that names the place, not the string in it. The server emitter would `Display`
//! the value instead; nothing on the client converts one yet.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Post {
    url: &'static str,
    title: &'static str,
}

/// A slice-backed loop: one row per item, each row its own template.
fn slice_rows(posts: &[Post]) -> view_abi::Node {
    dom_view_client_only! {
        <ul>
            for post in posts {
                <li><a href=(post.url)>(post.title)</a></li>
            }
        </ul>
    }
}

/// A `Vec`-backed loop over the same shape.
fn vec_rows(titles: &Vec<&'static str>) -> view_abi::Node {
    dom_view_client_only! {
        <ul>
            for title in titles {
                <li>(*title)</li>
            }
        </ul>
    }
}

/// A loop whose rows are themselves lists.
fn nested_rows(rows: &[&[&'static str]]) -> view_abi::Node {
    dom_view_client_only! {
        <table>
            <tbody>
                for row in rows {
                    <tr>
                        for cell in *row {
                            <td>(*cell)</td>
                        }
                    </tr>
                }
            </tbody>
        </table>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let posts = [Post { url: "/a", title: "A" }, Post { url: "/b", title: "B" }];
    let _ = slice_rows(&posts);

    let mut titles = Vec::new();
    titles.push("A");
    titles.push("B");
    let _ = vec_rows(&titles);

    let first: &[&'static str] = &["a", "b"];
    let second: &[&'static str] = &["c"];
    let rows: [&[&'static str]; 2] = [first, second];
    let _ = nested_rows(&rows);
}
