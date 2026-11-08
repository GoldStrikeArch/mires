//! Corpus family 04: which sink an event handler reaches.
//!
//! `@name` picks the sink from the same 22-name delegated set the reference plugin uses: `click`
//! is delegated and becomes a `$$click` property, `change` is not and becomes an `addEventListener`
//! call. The cases are `contract/fixtures/corpus/04-event-expressions/view.rs`, with handler bodies
//! kept to arithmetic.
//!
//! The handler parameters are written bare, exactly as the corpus fixture writes them: a handler
//! reaches the backend through `view_abi::handler`, whose bound is `Fn(&Event)`, and the emitter
//! annotates a bare parameter with that type. Before the ABI had a marker for handlers the value
//! went through the fully generic `hole<V>`, and every parameter had to be annotated by hand.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn delegated(count: u32) -> view_abi::Node {
    dom_view_client_only! {
        <button @click=$(|_e| count + 1)>"Click"</button>
    }
}

fn non_delegated(value: u32) -> view_abi::Node {
    dom_view_client_only! {
        <button @change=$(|_e| value)>"Change"</button>
    }
}

fn custom_event(value: u32) -> view_abi::Node {
    dom_view_client_only! {
        <button @custom-event=$(|_e| value)>"Custom"</button>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = delegated(1);
    let _ = non_delegated(2);
    let _ = custom_event(3);
}
