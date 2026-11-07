//! Family 15 -- `:name=$(expr)` bind attributes.
//!
//! LOWERS: `lower/bind_attribute.rs:9`. Only a dynamic bind-attribute NAME is
//! still refused (`:16`).
//!
//! A bind attribute is the reactive twin of `name=(expr)`: the server renders
//! the initial value into the template, and the browser re-applies it whenever a
//! signal the expression read changes. Both halves the emitter needed have
//! landed -- the reactive flag, which is true for `$(...)` and false for
//! `(...)`, and the per-name sink decision between `Attribute`, `Property`,
//! `ClassList` and `Style`.
//! Status per family: `contract/fixtures/l2-status.json`.

use view_dom_macro::dom_view;

fn bind_hidden() -> view_abi::Node {
    dom_view! {
        signal open = false;

        <p :hidden=$(!open.get())>"A fullstack Rust framework."</p>
    }
}

fn bind_value() -> view_abi::Node {
    dom_view! {
        signal name = String::new();

        <input :value=$(name.get())>
    }
}

fn bind_checked() -> view_abi::Node {
    dom_view! {
        signal done = false;

        <input type="checkbox" :checked=$(done.get())>
    }
}

fn bind_class() -> view_abi::Node {
    dom_view! {
        signal active = false;

        <div :class=$(if active.get() { "on" } else { "off" })></div>
    }
}

fn bind_style() -> view_abi::Node {
    dom_view! {
        signal color = String::new();

        <div :style=$(color.get())></div>
    }
}

fn bind_disabled() -> view_abi::Node {
    dom_view! {
        signal busy = false;

        <button :disabled=$(busy.get())>"Save"</button>
    }
}

fn two_way() -> view_abi::Node {
    dom_view! {
        signal name = String::new();

        <div>
            <input :value=$(name.get()) @input=$(|e| name.set(e.target.value))>
            <p>"Hello, " $(name.get()) "!"</p>
        </div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = bind_hidden;
    let _ = bind_value;
    let _ = bind_checked;
    let _ = bind_class;
    let _ = bind_style;
    let _ = bind_disabled;
    let _ = two_way;
}
