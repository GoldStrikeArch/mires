//! What a handler can ask its `Event`: the value of the element it came from, and cancelling the
//! browser's default.
//!
//! `view_abi::Event` is opaque, so before this an `@input` handler could be written but could not
//! read what the user typed, which is the one thing an `@input` handler exists for. Two markers fix
//! that, and they are the whole surface deliberately: anything else about a DOM event belongs to a
//! declared JavaScript interface rather than to the type that makes a closure's parameter nameable.
//!
//! There is no `.family` here. The corpus's event family is about which SINK a handler reaches
//! (family 04), and the reference compiler has no counterpart to this: JSX hands the handler the
//! DOM event and the author reads `e.target.value` in JavaScript, so there is nothing to compare
//! against.
//!
//! The emission is pinned by `21_event_accessors.js.expected`. What the emission MEANS is pinned by
//! `scripts/event-accessor-check.mjs`, which calls the two exported functions below with a fake
//! event: the dom suite's trace records that a handler was installed but never invokes one, so the
//! accessor's own behaviour is invisible to it. Same limitation, and same answer, as the keyed rows
//! in `20_keyed_for.rs`.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// The search island's shape: what the user typed goes into a signal on every keystroke.
fn search_box() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            signal query = "";
            <input @input=$(|e| query.set(e.target_value())) />
            <p>{ $(query.get()) }</p>
        </div>
    }
}

/// A form that does not navigate, which is the only thing `prevent_default` is for here.
fn form() -> view_abi::Node {
    dom_view_client_only! {
        <form @submit=$(|e| e.prevent_default())>
            <button>"Go"</button>
        </form>
    }
}

/// Called by scripts/event-accessor-check.mjs with a fake event.
#[unsafe(no_mangle)]
pub fn read_target_value(event: &view_abi::Event) -> &'static str {
    event.target_value()
}

/// Called by scripts/event-accessor-check.mjs with a fake event that records the call.
#[unsafe(no_mangle)]
pub fn cancel(event: &view_abi::Event) {
    event.prevent_default();
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = search_box();
    let _ = form();
}
