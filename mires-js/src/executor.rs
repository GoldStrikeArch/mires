//! A single-threaded executor for host-driven local futures.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use crate::{local_waker, microtask};

/// One local future and its scheduling state.
struct Task<F> {
    future: RefCell<Option<Pin<Box<F>>>>,
    scheduled: Cell<bool>,
    done: Cell<bool>,
}

/// One result future and the host functions that settle its Promise.
struct ResultTask<F> {
    future: RefCell<Option<Pin<Box<F>>>>,
    resolve: crate::JsValue,
    reject: crate::JsValue,
    scheduled: Cell<bool>,
    done: Cell<bool>,
}

impl<F> ResultTask<F>
where
    F: Future<Output = Result<crate::JsValue, crate::JsValue>> + 'static,
{
    /// Queues one poll, coalescing repeated wakes.
    fn schedule(self: &Rc<Self>) {
        if self.done.get() || self.scheduled.replace(true) {
            return;
        }
        let task = Rc::clone(self);
        microtask(move || task.poll());
    }

    /// Polls once, dropping the future before settling its Promise.
    fn poll(self: &Rc<Self>) {
        self.scheduled.set(false);
        if self.done.get() {
            return;
        }
        let task = Rc::clone(self);
        let waker = local_waker(move || task.schedule());
        let result = {
            let Ok(mut slot) = self.future.try_borrow_mut() else {
                return;
            };
            let Some(future) = slot.as_mut() else { return };
            let polled = Cell::new(Poll::Pending);
            let future = RefCell::new(future.as_mut());
            match crate::try_host(|| {
                polled.set(poll_result(future.borrow_mut().as_mut(), waker.clone()));
                crate::JsValue::undefined()
            }) {
                Err(error) => Err(error),
                Ok(_) => match polled.get() {
                    Poll::Pending => return,
                    Poll::Ready(result) => result,
                },
            }
        };
        self.done.set(true);
        // Remove the frame before calling drop: a throwing destructor must not leave it
        // available to a later wake. This protects the driver boundary, not Rust unwinding.
        let future = Cell::new(self.future.borrow_mut().take());
        let dropped = crate::try_host(|| {
            drop(future.take());
            crate::JsValue::undefined()
        });
        let (settle, value) = match dropped.and(result) {
            Ok(value) => (self.resolve, value),
            Err(value) => (self.reject, value),
        };
        let _ = crate::invoke(settle, crate::JsValue::undefined(), &[value]);
    }
}

impl<F> Task<F>
where
    F: Future<Output = ()> + 'static,
{
    /// Queues one poll, coalescing repeated wakes.
    fn schedule(self: &Rc<Self>) {
        if self.done.get() || self.scheduled.replace(true) {
            return;
        }
        let task = Rc::clone(self);
        microtask(move || task.poll());
    }

    /// Polls once and releases terminal state before propagating a host exception.
    fn poll(self: &Rc<Self>) {
        self.scheduled.set(false);
        if self.done.get() {
            return;
        }
        let task = Rc::clone(self);
        let waker = local_waker(move || task.schedule());
        let result = {
            let Ok(mut slot) = self.future.try_borrow_mut() else {
                return;
            };
            let Some(future) = slot.as_mut() else {
                return;
            };
            let ready = Cell::new(false);
            let future = RefCell::new(future.as_mut());
            let result = crate::try_host(|| {
                ready.set(poll_once(future.borrow_mut().as_mut(), waker.clone()));
                crate::JsValue::undefined()
            });
            if result.is_ok() && !ready.get() {
                return;
            }
            result
        };
        self.done.set(true);
        let future = Cell::new(self.future.borrow_mut().take());
        let dropped = crate::try_host(|| {
            drop(future.take());
            crate::JsValue::undefined()
        });
        if let Err(error) = dropped.and(result) {
            crate::throw_value(error);
        }
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
    let task = Rc::new(Task {
        done: Cell::new(false),
        future: RefCell::new(Some(Box::pin(future))),
        scheduled: Cell::new(false),
    });
    task.poll();
}

/// Converts one concrete future into a host Promise without an adapter future when optimized.
#[must_use]
pub(crate) fn promise_from_future<F>(future: F) -> crate::JsValue
where
    F: Future<Output = Result<crate::JsValue, crate::JsValue>> + 'static,
{
    host_promise_future(future)
}

/// The opt-in native-async continuation trial behind [`crate::future_to_native_promise`].
pub(crate) fn native_promise_from_future<F>(future: F) -> crate::JsValue
where
    F: Future<Output = Result<crate::JsValue, crate::JsValue>> + 'static,
{
    host_native_promise_future(future)
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.futurepromise")]
#[inline(never)]
fn host_promise_future<F>(future: F) -> crate::JsValue
where
    F: Future<Output = Result<crate::JsValue, crate::JsValue>> + 'static,
{
    crate::promise_new(move |resolve, reject| {
        let task = Rc::new(ResultTask {
            future: RefCell::new(Some(Box::pin(future))),
            resolve,
            reject,
            scheduled: Cell::new(false),
            done: Cell::new(false),
        });
        task.poll();
    })
}

#[cfg_attr(
    target_arch = "wasm32",
    link_section = "rcgjs.host.futurenativepromise"
)]
#[inline(never)]
fn host_native_promise_future<F>(future: F) -> crate::JsValue
where
    F: Future<Output = Result<crate::JsValue, crate::JsValue>> + 'static,
{
    // The ordinary Rust executor is the exact fallback whenever the compiler trial is disabled.
    host_promise_future(future)
}
