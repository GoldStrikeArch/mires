//! Awaiting a host Promise.

use alloc::rc::Rc;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use view_abi::{on_settled, JsValue};

/// How far along a [`Settling`] is.
enum State<V> {
    /// Nothing has been registered yet, so the first poll is the one that registers.
    Fresh,
    /// Registered and not yet settled.
    Waiting,
    /// Settled, with what it settled to and whether it succeeded.
    Settled(V, bool),
}

/// A host Promise, as a `Future`.
///
/// Lazy, in the sense Rust futures are and JavaScript Promises are not: nothing is registered
/// until the first poll. What that buys is the semantics the existing browser client already
/// models by hand -- a call that was never awaited never issued its request -- rather than a
/// second, eager shape sitting beside it.
pub struct Settling<V> {
    /// The value handed to [`await_promise`]. A thenable is awaited; anything else settles at
    /// once, so awaiting a plain value costs one microtask and works.
    promise: JsValue,
    /// Shared with the callback, which is the only thing that ever writes it.
    state: Rc<RefCell<State<V>>>,
}

/// Awaits `promise`, resolving to what it settled to.
///
/// `V` is the caller's claim about what the promise produces, exactly as it is on a signal read:
/// in this value model a JavaScript string IS a `&str`, so a promise of a string awaited as one
/// needs no conversion.
///
/// The rejection arrives as a `V` too, in the `Err`, because a JavaScript rejection carries any
/// value at all and only the interface that produced this promise knows which. What matters is
/// that it arrives INSIDE the poll: the pinned runtime's error handling is entirely synchronous,
/// so a rejection left as a rejection reaches no boundary and becomes a host-level unhandled
/// rejection (`CONTRACT-DOM` 14.4).
pub fn await_promise<V>(promise: JsValue) -> Settling<V> {
    Settling { promise, state: Rc::new(RefCell::new(State::Fresh)) }
}

impl<V> Future for Settling<V> {
    type Output = Result<V, V>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        // A handle and an `Rc`: nothing here cares where it lives, so the pin is not load bearing.
        let me = self.get_mut();
        let taken = core::mem::replace(&mut *me.state.borrow_mut(), State::Waiting);
        match taken {
            State::Fresh => {
                let state = Rc::clone(&me.state);
                let waker = context.waker().clone();
                on_settled(me.promise, move |value: V, ok: bool| {
                    *state.borrow_mut() = State::Settled(value, ok);
                    waker.wake_by_ref();
                });
                Poll::Pending
            }
            State::Waiting => Poll::Pending,
            State::Settled(value, ok) => Poll::Ready(match ok {
                true => Ok(value),
                false => Err(value),
            }),
        }
    }
}
