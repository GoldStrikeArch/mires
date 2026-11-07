//! `async fn` and `.await`: the coroutine state machine, lowered.
//!
//! A coroutine after rustc's `StateTransform` pass is an enum in everything but [`ty::TyKind`]:
//! its layout is `Variants::Multiple` with a tag, its states are variants, and the locals it holds
//! across a suspension are per-state fields. So it needs no machinery of its own, only the four
//! answers the value model owes any enum:
//!
//! * its JavaScript value is an object whose `TAG` is the state's INDEX (`EnumRepr::State`), since
//!   its states have no names to spell;
//! * with no state downcast a field is an upvar, and inside a state it is a saved local, keyed by
//!   the `CoroutineSavedLocal` the layout names rather than by the per-state field index: one local
//!   can sit at different indices in two states that both hold it, and keying by the index would
//!   make one local read as another after a resume;
//! * building one puts it in its unresumed state holding its upvars;
//! * changing its state moves the tag and leaves the saved locals where they are.
//!
//! There is **no executor here, deliberately**. `block_on` below polls by hand, which is what a
//! `__rt.spawn` with a microtask waker would do for it, and `Waker::noop` is what stands in for a
//! real waker. What this fixture pins is the lowering: that a state machine rustc built runs, and
//! that a local really does survive a suspension. Nothing here schedules anything, and per
//! CONTRACT-DOM section 14 an island's SETUP must stay synchronous whatever an executor later does.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Poll, Waker};

/// A leaf future that pends exactly once, so there is a real suspension rather than one the
/// optimizer can fold away.
struct PendOnce {
    polled: bool,
}

impl Future for PendOnce {
    type Output = i32;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<i32> {
        if self.polled {
            return Poll::Ready(7);
        }
        self.polled = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// No suspension point at all: the simplest coroutine shape.
async fn immediate(x: i32) -> i32 {
    x + 1
}

/// One `.await` of another `async fn`: the nested coroutine.
async fn one_await(x: i32) -> i32 {
    immediate(x).await * 2
}

/// Two awaits over a real suspension, so the machine has several states and has to keep a live
/// local across a yield. `a` is the local that must survive, and reading it back as anything else
/// is the silent wrong answer the saved-local key space exists to prevent.
async fn two_awaits(x: i32) -> i32 {
    let a = immediate(x).await;
    let b = PendOnce { polled: false }.await;
    a + b
}

/// A local of a type with no `Copy`, held across the suspension: the saved local is an object, so a
/// resume that read the wrong field would answer `undefined` rather than a wrong number.
async fn holds_a_struct(x: i32) -> i32 {
    let pair = Pair { left: x, right: x * 10 };
    let waited = PendOnce { polled: false }.await;
    pair.left + pair.right + waited
}

struct Pair {
    left: i32,
    right: i32,
}

/// Drives a future to completion by polling it in a loop.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => continue,
        }
    }
}

#[no_mangle]
pub fn rust_entry() {
    print_i32(block_on(immediate(1)));
    print_i32(block_on(one_await(2)));
    print_i32(block_on(two_awaits(3)));
    print_i32(block_on(holds_a_struct(4)));

    // Polling by hand, so the `Pending` arm is observed rather than looped over.
    let mut future = pin!(PendOnce { polled: false });
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => print_i32(value),
        Poll::Pending => print_str("pending"),
    }
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => print_i32(value),
        Poll::Pending => print_str("pending"),
    }
}
