//! Corpus family 15: `:name=$(expr)` bind attributes, the reactive twin of `name=(expr)`.
//!
//! The name decides the sink: `class` and `style` take the diffing helpers, a property name is
//! assigned, everything else is an attribute. Every one of them is reactive, so each is written
//! inside an effect, and two of them on one element share that effect (CONTRACT-DOM 6.2). The
//! cases are `contract/fixtures/corpus/15-topcoat-bind/view.rs`, with the `String` signals written
//! as `u32` so the fixture stays comparable with the reference trace.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn bind_hidden() -> view_abi::Node {
    dom_view_client_only! {
        signal open = false;

        <p :hidden=$(!open.get())>"A fullstack Rust framework."</p>
    }
}

fn bind_checked() -> view_abi::Node {
    dom_view_client_only! {
        signal done = false;

        <input type="checkbox" :checked=$(done.get())>
    }
}

fn bind_class() -> view_abi::Node {
    dom_view_client_only! {
        signal active = false;

        <div :class=$(if active.get() { "on" } else { "off" })></div>
    }
}

fn bind_style() -> view_abi::Node {
    dom_view_client_only! {
        signal color = 0u32;

        <div :style=$(color.get())></div>
    }
}

/// Two binds on one element: one effect, one previous-value record.
fn grouped() -> view_abi::Node {
    dom_view_client_only! {
        signal busy = false;

        <button :disabled=$(busy.get()) :title=$(busy.get())>"Save"</button>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = bind_hidden();
    let _ = bind_checked();
    let _ = bind_class();
    let _ = bind_style();
    let _ = grouped();
}
