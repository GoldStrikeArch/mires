//! A spawned future with no suspension runs ONCE, and in a microtask rather than inline.
//!
//! The two halves matter for different reasons. Once, because a task table that re-polled a
//! finished task would run a body twice and nothing above would notice. In a microtask, because
//! spawning is only ever reached from an event handler or an effect, and starting the body outside
//! whatever computation spawned it is what keeps a handler from owning work it did not ask for.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::say;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// No `.await` at all: the simplest coroutine there is.
async fn quiet() {
    say("body");
}

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    say("before");
    view_async::spawn(quiet());
    say("after");
}
