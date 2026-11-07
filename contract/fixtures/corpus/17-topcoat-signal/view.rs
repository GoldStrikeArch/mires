//! Family 17 -- signal declarations, `$()` reads, and `@event=$()` writes.
//!
//! LOWERS: `lower/signal_declaration.rs` claims its `KeySite::Signal` ordinal so
//! the sites after it are numbered the same for every emitter, and emits
//! `let <ident> = ::view_abi::signal(<ordinal>, <init>);`. The initializer is no
//! longer dropped, so the signal has storage: a `.get()` in a `$()` hole reads
//! it and a `.set()` in a handler writes it. `$(...)` reads and `@click=$(...)`
//! writes lower as a reactive `Child` hole and a `DelegatedEvent` hole.
//!
//! The family is EXCLUDED from the L2 comparison anyway, because the reference
//! module for family 17 is a different program. See NOTES.md and
//! `contract/fixtures/l2-status.json`.

use view_dom_macro::dom_view;

fn counter() -> view_abi::Node {
    dom_view! {
        signal count = 0.0;

        <div>
            <button @click=$(|_e| count.set(count.get() + 1.0))>"+1"</button>
            <p>"Count: " $(count.get())</p>
        </div>
    }
}

fn text_input() -> view_abi::Node {
    dom_view! {
        signal name = String::new();

        <div>
            <input @input=$(|e| name.set(e.target.value))>
            <p>"Hello, " $(name.get()) "!"</p>
        </div>
    }
}

fn toggle() -> view_abi::Node {
    dom_view! {
        signal open = false;

        <div>
            <button @click=$(|_e| open.set(!open.get()))>"What is Topcoat?"</button>
            <p>$(open.get())</p>
        </div>
    }
}

/// Two signals in one body: the `KeySite::Signal` ordinals must stay in
/// declaration order even though neither produces DOM.
fn two_signals() -> view_abi::Node {
    dom_view! {
        signal first = 0.0;
        signal second = 0.0;

        <div>
            <p>$(first.get())</p>
            <p>$(second.get())</p>
        </div>
    }
}

#[test]
fn the_expansion_type_checks() {
    let _ = counter;
    let _ = text_input;
    let _ = toggle;
    let _ = two_signals;
}
