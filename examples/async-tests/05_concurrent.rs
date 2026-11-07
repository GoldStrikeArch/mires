//! Two tasks in flight at once, resumed in the order their promises settle.
//!
//! The table is a table and not a slot: a second spawn must not evict the first, and a wake must
//! reach the task it named rather than whichever was polled last. The driver settles the two
//! promises in the opposite order to the spawns, so a table that answered the wrong task would
//! interleave the steps differently rather than merely lose one.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::{pending, say};

use view_abi::JsValue;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

async fn first(promise: JsValue) {
    say("first-start");
    let _: Result<&'static str, &'static str> = view_async::await_promise(promise).await;
    say("first-end");
}

async fn second(promise: JsValue) {
    say("second-start");
    let _: Result<&'static str, &'static str> = view_async::await_promise(promise).await;
    say("second-end");
}

/// Spawns both over the two promises the driver holds, which it settles second-then-first.
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    view_async::spawn(first(pending()));
    view_async::spawn(second(pending()));
}

/// A value that is not a thenable at all, awaited: it settles at once, in a microtask, so
/// awaiting a plain value works and costs nothing but a turn.
#[unsafe(no_mangle)]
pub extern "C" fn run_plain() {
    view_async::spawn(async {
        let value: Result<&'static str, &'static str> =
            view_async::await_promise(prelude::plain("here")).await;
        say(match value {
            Ok(value) => value,
            Err(_) => "rejected",
        });
    });
}
