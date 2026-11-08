//! Promise and browser-task futures.

use alloc::rc::Rc;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use crate::{native_promise_from_future, on_settled, promise_from_future, task, JsValue};

/// How far a host value has settled.
enum State {
    Fresh,
    Waiting,
    Settled(JsValue, bool),
}

/// A host Promise or thenable as a Rust future.
pub struct PromiseFuture {
    value: JsValue,
    state: Rc<RefCell<State>>,
}

impl Future for PromiseFuture {
    type Output = Result<JsValue, JsValue>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let me = self.get_mut();
        let state = core::mem::replace(&mut *me.state.borrow_mut(), State::Waiting);
        match state {
            State::Fresh => {
                let subscribed = crate::try_host(|| {
                    let shared = Rc::clone(&me.state);
                    let waker = context.local_waker().clone();
                    on_settled(me.value, move |value, ok| {
                        *shared.borrow_mut() = State::Settled(value, ok);
                        waker.wake_by_ref();
                    });
                    JsValue::undefined()
                });
                if let Err(error) = subscribed {
                    return Poll::Ready(Err(error));
                }
                Poll::Pending
            }
            State::Waiting => Poll::Pending,
            State::Settled(value, ok) => Poll::Ready(if ok { Ok(value) } else { Err(value) }),
        }
    }
}

/// Awaits a Promise, thenable, or plain host value.
///
/// An exception thrown while subscribing is returned as `Err` by the first poll, including
/// when a thenable calls a settlement callback before throwing. Normal settlement resumes on
/// a later poll. Catching subscription errors lets the awaiting Rust future run its cleanup.
#[must_use]
pub fn await_promise(value: JsValue) -> PromiseFuture {
    PromiseFuture {
        value,
        state: Rc::new(RefCell::new(State::Fresh)),
    }
}

/// Converts a Rust future into a host Promise.
///
/// Host exceptions at the driver's poll or drop boundary reject this Promise with the thrown
/// value. The driver removes the future before dropping it and ignores later wakes. This does
/// not provide Rust unwinding or reconstruct locals lost when an exception exits a poll.
#[must_use]
pub fn future_to_promise<F>(future: F) -> JsValue
where
    F: Future<Output = Result<JsValue, JsValue>> + 'static,
{
    promise_from_future(future)
}

/// Converts a Rust future with an explicit native-async continuation driver.
///
/// This is a focused compiler trial. It retains the Rust future state machine and its exact poll
/// and drop callbacks, but schedules later polls through one JavaScript `async` function instead
/// of the default queued-microtask driver. [`future_to_promise`] remains the production baseline.
#[must_use]
pub fn future_to_native_promise<F>(future: F) -> JsValue
where
    F: Future<Output = Result<JsValue, JsValue>> + 'static,
{
    native_promise_from_future(future)
}

/// A future that yields to one browser task.
pub struct YieldTask {
    yielded: bool,
}

impl Future for YieldTask {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            return Poll::Ready(());
        }
        self.yielded = true;
        let waker = context.local_waker().clone();
        task(move || waker.wake_by_ref());
        Poll::Pending
    }
}

/// Yields once through a fresh `MessageChannel` task.
#[must_use]
pub fn yield_task() -> YieldTask {
    YieldTask { yielded: false }
}
