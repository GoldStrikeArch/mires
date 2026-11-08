//! The owner-aware local future driver.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use view_abi::{microtask, on_cleanup, owner, with_owner, Owner};

/// One future used when optimizer lowering is disabled.
struct Task<F> {
    future: RefCell<Option<Pin<Box<F>>>>,
    owner: Owner,
    scheduled: Cell<bool>,
    cancelled: Cell<bool>,
}

impl<F> Task<F>
where
    F: Future<Output = ()> + 'static,
{
    /// Queues one owner-aware poll, coalescing repeated wakes.
    fn schedule(self: &Rc<Self>) {
        if self.cancelled.get() || self.scheduled.replace(true) {
            return;
        }
        let task = Rc::clone(self);
        microtask(move || task.poll());
    }

    /// Re-enters the captured owner and polls once.
    fn poll(self: &Rc<Self>) {
        self.scheduled.set(false);
        if self.cancelled.get() {
            return;
        }
        let task = Rc::clone(self);
        let waker = local_waker(move || task.schedule());
        let task = Rc::clone(self);
        with_owner(self.owner, move || {
            let result = {
                let Ok(mut slot) = task.future.try_borrow_mut() else {
                    return;
                };
                let Some(future) = slot.as_mut() else {
                    return;
                };
                let ready = Cell::new(false);
                let future = RefCell::new(future.as_mut());
                let result = view_abi::try_host::<view_abi::JsValue, _>(|| {
                    ready.set(poll_once(future.borrow_mut().as_mut(), waker.clone()));
                });
                if result.is_ok() && !ready.get() && !task.cancelled.get() {
                    return;
                }
                result
            };
            task.cancelled.set(true);
            let future = Cell::new(task.future.borrow_mut().take());
            let dropped = view_abi::try_host(|| drop(future.take()));
            if let Err(error) = dropped.and(result) {
                view_abi::throw_host(error);
            }
        });
    }

    /// Drops suspended state under its owner, or defers the drop until an active poll returns.
    fn cancel(self: &Rc<Self>) {
        if self.cancelled.replace(true) {
            return;
        }
        let task = Rc::clone(self);
        with_owner(self.owner, move || {
            if let Ok(mut slot) = task.future.try_borrow_mut() {
                let future = slot.take();
                drop(slot);
                drop(future);
            }
        });
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

/// Runs `future` to completion, resuming it in owner-aware microtasks.
///
/// The owner is captured before the deferred first poll and re-entered for every resume.
/// Cleaning up that owner drops the future and ignores later wakes. If cleanup happens
/// during a poll, the future is dropped when that poll returns. Already-issued host work
/// is not aborted by dropping its waiting future. A host exception from polling also stops
/// the task and drops its future under the captured owner before propagating. A drop exception
/// takes precedence. This does not unwind locals inside the future's poll method.
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    host_spawn(future, owner());
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.spawnowner.v2")]
#[inline(never)]
fn host_spawn<F>(future: F, owner: Owner)
where
    F: Future<Output = ()> + 'static,
{
    let task = Rc::new(Task {
        future: RefCell::new(Some(Box::pin(future))),
        owner,
        scheduled: Cell::new(false),
        cancelled: Cell::new(false),
    });
    let cleanup = Rc::clone(&task);
    on_cleanup(move || cleanup.cancel());
    task.schedule();
}

/// Creates a safe callback-backed waker for the fallback executor.
fn local_waker<F>(f: F) -> Waker
where
    F: Fn() + 'static,
{
    host_local_waker(f)
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.localwaker")]
#[inline(never)]
fn host_local_waker<F>(f: F) -> Waker
where
    F: Fn(),
{
    f();
    Waker::noop().clone()
}
