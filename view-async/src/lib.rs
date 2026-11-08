//! The executor: what runs a Rust `Future` inside a compiled island.
//!
//! `view-abi` gives the Promise and owner primitives. An optimized build uses the
//! compiler's monomorphic task driver; the baseline keeps the same policy in safe Rust.
//! This crate adds owner policy plus a leaf Promise future.
//!
//! # Why the loop is here and not in the shim
//!
//! In optimized builds, the backend passes each concrete future's poll and drop
//! functions to one small JavaScript scheduler. Rust semantics stay in monomorphized
//! Rust functions while scheduling state stays in the host event loop.
//!
//! # What an island may do with it
//!
//! **Only an event handler or an effect may spawn.** An island's setup is one
//! synchronous call stack: the runtime clears the hydration context in a synchronous
//! `finally`, so a continuation resuming after it finds no context, builds fresh DOM
//! instead of claiming the server's, and reports nothing in a production build
//! (`CONTRACT-DOM` 14.6). A delegated event arriving during the gap ends hydration
//! for the whole document (14.8). Neither is a failure this crate can detect, and
//! both are silent, which is why the constraint is stated here rather than checked.
//!
//! Other limits come from the host boundary:
//!
//! * a `batch` closes at the first suspension point, so two signal writes in a continuation are two
//!   update passes (14.5);
//! * owner cleanup drops its pending futures and stops their later resumptions, but does not abort
//!   already-issued host work such as a fetch or a Promise;
//! * a future is never resolved during hydration, which is the same rule as the first one seen from
//!   the other side.

#![no_std]

extern crate alloc;

mod promise;
mod task;

pub use promise::*;
pub use task::*;
