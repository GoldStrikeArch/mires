//! A future that awaits a resolved promise and then writes a signal, inside the owner the spawn
//! captured.
//!
//! The owner is the whole point. The runtime's `Owner` is a module-level global restored in a
//! synchronous `finally`, so for a suspended computation that `finally` fires at the FIRST
//! suspension: by the time a continuation runs there is no owner, a signal read there does not
//! subscribe, an `onCleanup` is a no-op and an effect created there is never disposed
//! (`CONTRACT-DOM` 14.3). Every one of those is silent. So the executor captures the owner before
//! the first poll and re-enters it on every resume, and this fixture records the owner at both
//! ends so the driver can assert they are the same object.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::{note_owner, resolved, say};

use view_abi::Sig;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

async fn body(count: Sig<f64>) {
    let reply: Result<&'static str, &'static str> =
        view_async::await_promise(resolved("reply")).await;
    note_owner("resume", view_abi::owner());
    say(match reply {
        Ok(value) => value,
        Err(_) => "rejected",
    });
    // The write the whole path exists for: a value came back and the view is told.
    count.set(count.get() + 1.0);
}

/// Answers the signal so the driver holds the same `[read, write]` pair the future writes.
#[unsafe(no_mangle)]
pub extern "C" fn run() -> Sig<f64> {
    let count = view_abi::signal(0, 0.0);
    note_owner("spawn", view_abi::owner());
    view_async::spawn(body(count));
    count
}
