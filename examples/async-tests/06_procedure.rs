//! The integration fixture: a procedure called from compiled client code.
//!
//! Everything below the `#[procedure(serde)]` line is the FRAMEWORK'S, unmodified. The attribute
//! is `topcoat_runtime_macro::procedure`, compiled from `crates/topcoat-runtime/macro`, and this
//! crate is built with `--cfg topcoat_client`, so what rustc sees is the client expansion the
//! framework already ships:
//!
//! ```text
//! async fn greet(name: String) -> ::topcoat_dom::Result<<Result<String> as ::topcoat_dom::ResultExt>::T> {
//!     let __topcoat_reply = ::topcoat_dom::procedure::call(ID, &::topcoat_dom::json::to_json(&(&name,))?).await?;
//!     ::topcoat_dom::json::from_json(&__topcoat_reply)
//! }
//! ```
//!
//! That expansion has been landed and unrunnable since it was written, because `procedure::call`
//! is awaitable and nothing could drive a `Future`. The question this fixture answers is whether
//! the executor closes it: does the framework's own expansion compile and run, as written, with no
//! macro change?
//!
//! # The shape of the call, which is the shape a handler makes
//!
//! `run` is what an `@click` handler's body is: it spawns, and returns. The spawn is what keeps
//! the suspension out of the caller's stack, which is the whole constraint -- an island's setup is
//! one synchronous pass and a call that suspended inside it would silently degrade hydration to a
//! client render (`CONTRACT-DOM` 14.6). The answer is written into a signal, because a signal is
//! how a view is told anything.
//!
//! `scripts/async-check.mjs` installs `topcoatFetch` and asserts the wire the compiled program
//! produced: the route, the media type, and the argument array.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::say;

use alloc::string::String;
use topcoat_dom::Result;
use topcoat_runtime_macro::procedure;
use view_abi::Sig;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// One argument, so the expansion's `to_json(&(&name,))` path is the one compiled.
///
/// The body is the SERVER's and is compiled away by the attribute's own `cfg`; it is here because
/// a procedure is one item with two halves and this is how the framework's users write it.
#[procedure(serde)]
async fn greet(name: String) -> Result<String> {
    Ok(name)
}

/// No arguments, so the expansion's `"[]"` literal is the one compiled. An empty argument list has
/// to be an empty ARRAY and not `null`, which is the asymmetry `contract/PROCEDURES.md` calls
/// deliberate and unshared.
#[procedure(serde)]
async fn version() -> Result<String> {
    Ok(String::from("5"))
}

/// A handler's body: spawn, write the answer into the signal, return.
#[unsafe(no_mangle)]
pub extern "C" fn on_click(name: &str, reply: Sig<&'static str>) {
    let name = String::from(name);
    view_async::spawn(async move {
        match greet(name).await {
            Ok(answer) => {
                say("ok");
                // The value the view reads. `display_to_str` is the existing crossing: it writes
                // through the host string builder, so what comes back is already the JavaScript
                // string a signal hands the runtime.
                reply.set(view_abi::display_to_str(&answer));
            }
            Err(error) => {
                say("err");
                reply.set(view_abi::display_to_str(error.message()));
            }
        }
    });
}

/// The zero-argument call, driven the same way.
#[unsafe(no_mangle)]
pub extern "C" fn on_version(reply: Sig<&'static str>) {
    view_async::spawn(async move {
        match version().await {
            Ok(answer) => reply.set(view_abi::display_to_str(&answer)),
            Err(error) => reply.set(view_abi::display_to_str(error.message())),
        }
    });
}

/// The signal the driver reads, made here so it is the runtime's own `[read, write]` pair.
#[unsafe(no_mangle)]
pub extern "C" fn make_signal() -> Sig<&'static str> {
    view_abi::signal(0, "")
}
