//! Family 13 -- `for pat in expr` with a markup body.
//!
//! LOWERS: `lower/node.rs:25-28` makes `for pat in expr` a reactive child hole.
//! Only the attribute-position form is still refused (`lower/attributes.rs:41`),
//! as is a dynamic attribute name (`lower/attribute.rs:13`), so `attr_for` needs
//! two features. Status per family: `contract/fixtures/l2-status.json`.
//!
//! Topcoat's `for` is a Rust loop over an iterator, evaluated once per render.
//! It is NOT Solid's keyed `<For>`: there is no per-item reconciliation and no
//! key, so the two are equivalent only for the first render.

use view_dom_macro::dom_view;

struct Post {
    url: &'static str,
    title: &'static str,
}

fn simple_for(posts: Vec<Post>) -> view_abi::Node {
    dom_view! {
        <ul>
            for post in posts {
                <li>
                    <a href=(post.url)>(post.title)</a>
                </li>
            }
        </ul>
    }
}

fn nested_for(rows: Vec<Vec<&'static str>>) -> view_abi::Node {
    dom_view! {
        <table>
            <tbody>
                for row in rows {
                    <tr>
                        for cell in row {
                            <td>(cell)</td>
                        }
                    </tr>
                }
            </tbody>
        </table>
    }
}

/// `for` in an attribute list emits zero or more ATTRIBUTES. This has no JSX
/// counterpart at all -- the nearest thing is spreading a built object, which
/// is a different construct with different evaluation order.
fn attr_for(attrs: Vec<(&'static str, &'static str)>) -> view_abi::Node {
    dom_view! {
        <div
            for (name, value) in attrs {
                (name)=(value)
            }
        ></div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = simple_for;
    let _ = nested_for;
    let _ = attr_for;
}
