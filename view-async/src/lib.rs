//! The executor: what runs a Rust `Future` inside a compiled island.
//!
//! `view-abi` gives four primitives -- a microtask, a promise bridge, and the two
//! halves of an owner capture -- and this crate is the policy built on them: a task
//! table, a poll loop, and a leaf future for a host Promise.
//!
//! # Why the loop is here and not in the shim
//!
//! A poll loop written in JavaScript has to call `Future::poll`, which is
//! monomorphized per future type and takes a `Pin<&mut Self>`. Nothing in the value
//! model hands out either a monomorphized method as a callable or a `&mut F` as its
//! receiver. Putting the loop in Rust shrinks the boundary to four primitives, none
//! of them generic over a future, and puts the policy where it can be tested.
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
//! Three further limits come from the same place and are not defects to be fixed:
//!
//! * a `batch` closes at the first suspension point, so two signal writes in a
//!   continuation are two update passes (14.5);
//! * dropping a future does not cancel its work, because nothing in the pinned
//!   runtime cancels anything and a host Promise has no counterpart (14.11);
//! * a future is never resolved during hydration, which is the same rule as the
//!   first one seen from the other side.

#![no_std]

extern crate alloc;

mod promise;
mod task;

pub use promise::*;
pub use task::*;
