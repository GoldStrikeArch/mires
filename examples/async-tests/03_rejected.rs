//! A rejected promise arrives as an `Err` INSIDE the poll, and never as a host rejection.
//!
//! The pinned runtime's only error path is a synchronous `try`/`catch`: `catchError` and
//! `ErrorBoundary` are both built on one, so a rejected promise reaches neither and becomes a
//! host-level unhandled rejection (`CONTRACT-DOM` 14.4). An executor that let a rejection stay a
//! rejection would be handing errors to nobody. The driver watches for `unhandledRejection` while
//! this runs, and the fixture records which arm the value came back through.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::{rejected, resolved, say};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

async fn body(promise: view_abi::JsValue) {
    let settled: Result<&'static str, &'static str> = view_async::await_promise(promise).await;
    match settled {
        Ok(value) => say(value),
        Err(reason) => say(reason),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn run_rejected() {
    view_async::spawn(body(rejected("boom")));
}

/// The same body against a promise that keeps its word, so the two arms are told apart by which
/// value arrives rather than by which was expected.
#[unsafe(no_mangle)]
pub extern "C" fn run_resolved() {
    view_async::spawn(body(resolved("fine")));
}
