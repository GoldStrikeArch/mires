//! A single-threaded executor for host-driven local futures.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use crate::{local_waker, microtask, JsValue};

/// Scheduling lifecycle of one [`Task`].
#[derive(Clone, Copy, Eq, PartialEq)]
enum TaskState {
    Idle,
    Scheduled,
    Done,
}

/// How a [`Task`] polls its concrete future and delivers its terminal outcome.
trait Settle<T> {
    fn poll<F: Future<Output = T>>(future: Pin<&mut F>, waker: Waker) -> Poll<T>;

    fn settle(&self, outcome: Result<T, JsValue>);
}

/// Settlement for [`spawn_local`]: rethrows a poll or drop exception on the host.
struct Spawn;

impl Settle<()> for Spawn {
    fn poll<F: Future<Output = ()>>(future: Pin<&mut F>, waker: Waker) -> Poll<()> {
        if poll_once(future, waker) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    fn settle(&self, outcome: Result<(), JsValue>) {
        if let Err(error) = outcome {
            crate::throw_value(error);
        }
    }
}

/// Settlement for [`promise_from_future`]: resolves or rejects the host Promise.
struct PromiseSettle {
    resolve: JsValue,
    reject: JsValue,
}

impl Settle<Result<JsValue, JsValue>> for PromiseSettle {
    fn poll<F: Future<Output = Result<JsValue, JsValue>>>(
        future: Pin<&mut F>,
        waker: Waker,
    ) -> Poll<Result<JsValue, JsValue>> {
        poll_result(future, waker)
    }

    fn settle(&self, outcome: Result<Result<JsValue, JsValue>, JsValue>) {
        let (settle, value) = match outcome.and_then(|result| result) {
            Ok(value) => (self.resolve, value),
            Err(value) => (self.reject, value),
        };
        let _ = crate::invoke(settle, JsValue::undefined(), &[value]);
    }
}

/// One local future, its scheduling state, and its terminal settlement handler.
struct Task<F, S> {
    future: RefCell<Option<Pin<Box<F>>>>,
    state: Cell<TaskState>,
    settle: S,
}

impl<F, S> Task<F, S>
where
    F: Future + 'static,
    S: Settle<F::Output> + 'static,
{
    fn new(future: F, settle: S) -> Rc<Self> {
        Rc::new(Self {
            future: RefCell::new(Some(Box::pin(future))),
            state: Cell::new(TaskState::Idle),
            settle,
        })
    }

    /// Queues one poll, coalescing repeated wakes.
    fn schedule(self: &Rc<Self>) {
        if self.state.get() != TaskState::Idle {
            return;
        }
        self.state.set(TaskState::Scheduled);
        let task = Rc::clone(self);
        microtask(move || task.poll());
    }

    /// Polls once, dropping the future before settling or propagating a host exception.
    fn poll(self: &Rc<Self>) {
        if self.state.get() == TaskState::Done {
            return;
        }
        self.state.set(TaskState::Idle);
        let task = Rc::clone(self);
        let waker = local_waker(move || task.schedule());
        let outcome = {
            let Ok(mut slot) = self.future.try_borrow_mut() else {
                return;
            };
            let Some(future) = slot.as_mut() else {
                return;
            };
            let polled = Cell::new(Poll::Pending);
            let future = RefCell::new(future.as_mut());
            match crate::try_host(|| {
                polled.set(S::poll(future.borrow_mut().as_mut(), waker.clone()));
                JsValue::undefined()
            }) {
                Err(error) => Err(error),
                Ok(_) => match polled.replace(Poll::Pending) {
                    Poll::Pending => return,
                    Poll::Ready(output) => Ok(output),
                },
            }
        };
        self.state.set(TaskState::Done);
        // Remove the frame before calling drop: a throwing destructor must not leave it
        // available to a later wake. This protects the driver boundary, not Rust unwinding.
        let future = Cell::new(self.future.borrow_mut().take());
        let dropped = crate::try_host(|| {
            drop(future.take());
            JsValue::undefined()
        });
        self.settle.settle(dropped.and(outcome));
    }
}

/// Polls one concrete future. The backend passes this function directly to the host driver.
fn poll_once<F>(mut future: Pin<&mut F>, waker: Waker) -> bool
where
    F: Future<Output = ()>,
{
    matches!(
        future.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Ready(())
    )
}

/// Polls one concrete result future. Its enum value is passed straight to the host driver.
fn poll_result<F>(mut future: Pin<&mut F>, waker: Waker) -> Poll<F::Output>
where
    F: Future,
{
    future.as_mut().poll(&mut Context::from_waker(&waker))
}

/// Runs a local future to completion.
///
/// The first poll is synchronous, matching a JavaScript async function and a `Promise` executor.
/// Later wakes are coalesced into microtasks. A host exception from polling stops the task
/// and drops its future before propagating. A drop exception takes precedence. This protects
/// the driver boundary; it does not unwind locals inside the future's poll method.
pub fn spawn_local<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    host_spawn(future);
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.spawn")]
#[inline(never)]
fn host_spawn<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    Task::new(future, Spawn).poll();
}

/// Converts one concrete future into a host Promise without an adapter future when optimized.
#[must_use]
pub(crate) fn promise_from_future<F>(future: F) -> JsValue
where
    F: Future<Output = Result<JsValue, JsValue>> + 'static,
{
    host_promise_future(future)
}

/// The opt-in native-async continuation trial behind [`crate::future_to_native_promise`].
pub(crate) fn native_promise_from_future<F>(future: F) -> JsValue
where
    F: Future<Output = Result<JsValue, JsValue>> + 'static,
{
    host_native_promise_future(future)
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.futurepromise")]
#[inline(never)]
fn host_promise_future<F>(future: F) -> JsValue
where
    F: Future<Output = Result<JsValue, JsValue>> + 'static,
{
    crate::promise_new(move |resolve, reject| {
        Task::new(future, PromiseSettle { resolve, reject }).poll();
    })
}

#[cfg_attr(
    target_arch = "wasm32",
    link_section = "rcgjs.host.futurenativepromise"
)]
#[inline(never)]
fn host_native_promise_future<F>(future: F) -> JsValue
where
    F: Future<Output = Result<JsValue, JsValue>> + 'static,
{
    // The ordinary Rust executor is the exact fallback whenever the compiler trial is disabled.
    host_promise_future(future)
}
