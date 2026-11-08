//! Corpus family 17: a signal, the handlers that write it and the holes that read it.
//!
//! This is the counter island's client half: `signal count = 0;`, an increment and a decrement
//! `@click` handler, a reactive text hole, and a bind attribute the same signal drives. Between
//! them they exercise every marker the reactive path has -- `signal`, `sget`, `sset`, a delegated
//! event hole, a reactive `Child` hole and a reactive `Property` hole -- and the trace this
//! produces is what a browser run of the island is checked against.
//!
//! There is no `.family` beside this fixture. The reference module for corpus family 17 is a
//! different program, so a trace comparison against it would not be measuring anything.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

fn counter() -> view_abi::Node {
    dom_view_client_only! {
        signal count = 0u32;

        <div>
            <button @click=$(|_e: &view_abi::Event| count.set(count.get() + 1))>"+1"</button>
            <button
                :disabled=$(count.get() == 0)
                @click=$(|_e: &view_abi::Event| count.set(count.get() - 1))
            >
                "-1"
            </button>
            <p>"Count: " $(count.get())</p>
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = counter();
}
