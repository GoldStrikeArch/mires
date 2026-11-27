//! Promise and browser-task futures.

use alloc::rc::Rc;
use core::cell::Cell;
use core::future::{Future, IntoFuture};
use core::pin::Pin;
use core::task::{Context, LocalWaker, Poll};

use crate::{
    js_bindings, native_promise_from_future, on_settled, promise_from_future, task, HostHandle,
    HostValue, JsValue,
};

/// A native JavaScript `Promise`.
///
/// Wraps an opaque host `Promise` object as one transparent handle word. Implements
/// [`IntoFuture`] so a `Promise` can be `.await`ed directly inside Rust `async` functions, and
/// [`Promise::from_future`] converts a Rust `Future` into a host `Promise`.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Promise {
    #[allow(dead_code, reason = "opaque host handle lowered by backend")]
    handle: HostHandle,
}

impl crate::host::sealed::Sealed for Promise {}
impl HostValue for Promise {
    fn accepts(value: JsValue) -> bool {
        value.is_object() && is_prototype_of(promise_prototype(), value)
    }
}

impl Promise {
    /// Creates a `Promise` resolved with `value` (`Promise.resolve(value)`).
    #[must_use]
    pub fn resolve(value: JsValue) -> Self {
        promise_resolve(value)
    }

    /// Creates a `Promise` rejected with `reason` (`Promise.reject(reason)`).
    #[must_use]
    pub fn reject(reason: JsValue) -> Self {
        promise_reject(reason)
    }

    /// Converts a Rust future into a native JavaScript `Promise`.
    #[must_use]
    pub fn from_future<F>(future: F) -> Self
    where
        F: Future<Output = Result<JsValue, JsValue>> + 'static,
    {
        future_to_promise(future).read()
    }

    /// Erases this `Promise` handle to an opaque [`JsValue`].
    #[must_use]
    pub fn as_value(self) -> JsValue {
        JsValue::erase(self)
    }
}

impl From<Promise> for JsValue {
    fn from(value: Promise) -> Self {
        value.as_value()
    }
}

impl TryFrom<JsValue> for Promise {
    type Error = JsValue;

    fn try_from(value: JsValue) -> Result<Self, Self::Error> {
        value.try_read()
    }
}

impl IntoFuture for Promise {
    type Output = Result<JsValue, JsValue>;
    type IntoFuture = PromiseFuture;

    fn into_future(self) -> Self::IntoFuture {
        await_promise(self.as_value())
    }
}

#[cfg(test)]
impl Promise {
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self {
            handle: HostHandle::from_raw(raw),
        }
    }

    pub(crate) const fn raw(self) -> u32 {
        self.handle.raw()
    }
}

/// A native JavaScript `Promise` whose settled value carries a static Rust type `T`.
///
/// At run time this is `#[repr(transparent)]` over [`Promise`] (zero runtime overhead — it is the
/// host `Promise` object itself). The compiler's `.d.ts` generator inspects `T` to emit
/// `Promise<T>` (unwrapping `Result<OkT, ErrT>` to `Promise<OkT>` with `@throws {ErrT}`, and `()`
/// to `Promise<void>`).
#[repr(transparent)]
pub struct TypedPromise<T> {
    inner: Promise,
    _marker: core::marker::PhantomData<T>,
}

impl<T> Clone for TypedPromise<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for TypedPromise<T> {}

impl<T> TypedPromise<T> {
    /// Wraps an untyped host [`Promise`] as a [`TypedPromise<T>`].
    #[must_use]
    pub const fn from_promise(promise: Promise) -> Self {
        Self {
            inner: promise,
            _marker: core::marker::PhantomData,
        }
    }

    /// Unwraps this typed promise into its underlying [`Promise`].
    #[must_use]
    pub const fn into_promise(self) -> Promise {
        self.inner
    }

    /// Converts a Rust future into a [`TypedPromise<T>`].
    #[must_use]
    pub fn from_future<F>(future: F) -> Self
    where
        F: Future<Output = Result<JsValue, JsValue>> + 'static,
    {
        Self::from_promise(Promise::from_future(future))
    }

    /// Erases this `TypedPromise` handle to an opaque [`JsValue`].
    #[must_use]
    pub fn as_value(self) -> JsValue {
        self.inner.as_value()
    }
}

impl<T> From<TypedPromise<T>> for Promise {
    fn from(value: TypedPromise<T>) -> Self {
        value.into_promise()
    }
}

impl<T> From<TypedPromise<T>> for JsValue {
    fn from(value: TypedPromise<T>) -> Self {
        value.as_value()
    }
}

js_bindings! {
    #[js(call = "Promise.resolve")]
    fn promise_resolve(value: JsValue) -> Promise;

    #[js(call = "Promise.reject")]
    fn promise_reject(reason: JsValue) -> Promise;

    #[js(global, get = "Promise.prototype")]
    fn promise_prototype() -> JsValue;

    #[js(method = "isPrototypeOf")]
    fn is_prototype_of(prototype: JsValue, value: JsValue) -> bool;
}

/// How far a host value has settled.
enum PromiseState {
    Fresh,
    Waiting(LocalWaker),
    Settled(Result<JsValue, JsValue>),
    Done,
}

/// A host Promise or thenable as a Rust future.
pub struct PromiseFuture {
    value: JsValue,
    state: Rc<Cell<PromiseState>>,
}

#[inline]
fn settle_promise_state(state_cell: &Cell<PromiseState>, outcome: Result<JsValue, JsValue>) {
    match state_cell.replace(PromiseState::Done) {
        PromiseState::Waiting(waker) => {
            state_cell.set(PromiseState::Settled(outcome));
            waker.wake();
        }
        other => state_cell.set(other),
    }
}

#[inline]
fn poll_promise_state<S>(
    state_cell: &Rc<Cell<PromiseState>>,
    context: &mut Context<'_>,
    subscribe: S,
) -> Poll<Result<JsValue, JsValue>>
where
    S: FnOnce(Rc<Cell<PromiseState>>) -> Result<(), JsValue>,
{
    match state_cell.replace(PromiseState::Done) {
        PromiseState::Fresh => {
            state_cell.set(PromiseState::Waiting(context.local_waker().clone()));
            if let Err(error) = subscribe(Rc::clone(state_cell)) {
                state_cell.set(PromiseState::Done);
                return Poll::Ready(Err(error));
            }
            Poll::Pending
        }
        PromiseState::Waiting(mut waker) => {
            waker.clone_from(context.local_waker());
            state_cell.set(PromiseState::Waiting(waker));
            Poll::Pending
        }
        PromiseState::Settled(outcome) => Poll::Ready(outcome),
        PromiseState::Done => panic!("PromiseFuture polled after completion"),
    }
}

impl Future for PromiseFuture {
    type Output = Result<JsValue, JsValue>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let me = self.get_mut();
        let value = me.value;
        poll_promise_state(&me.state, context, |shared| {
            crate::try_host(|| {
                let shared = Rc::clone(&shared);
                on_settled(value, move |settled, ok| {
                    let outcome = if ok { Ok(settled) } else { Err(settled) };
                    settle_promise_state(&shared, outcome);
                });
                JsValue::undefined()
            })
            .map(|_| ())
        })
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
        state: Rc::new(Cell::new(PromiseState::Fresh)),
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

/// Shared completion state between a [`YieldTask`] and its scheduled browser task.
enum YieldSlot {
    Waiting(LocalWaker),
    Fired,
}

/// Lifecycle of a [`YieldTask`].
enum YieldState {
    Fresh,
    Waiting(Rc<Cell<YieldSlot>>),
    Done,
}

/// A future that yields to one browser task.
pub struct YieldTask {
    state: YieldState,
}

#[inline]
fn fire_yield_slot(slot: &Cell<YieldSlot>) {
    if let YieldSlot::Waiting(waker) = slot.replace(YieldSlot::Fired) {
        waker.wake();
    }
}

#[inline]
fn poll_yield_state<S>(state: &mut YieldState, context: &mut Context<'_>, schedule: S) -> Poll<()>
where
    S: FnOnce(Rc<Cell<YieldSlot>>),
{
    match state {
        YieldState::Fresh => {
            let slot = Rc::new(Cell::new(YieldSlot::Waiting(context.local_waker().clone())));
            *state = YieldState::Waiting(Rc::clone(&slot));
            schedule(slot);
            Poll::Pending
        }
        YieldState::Waiting(slot) => match slot.replace(YieldSlot::Fired) {
            YieldSlot::Waiting(mut waker) => {
                waker.clone_from(context.local_waker());
                slot.set(YieldSlot::Waiting(waker));
                Poll::Pending
            }
            YieldSlot::Fired => {
                *state = YieldState::Done;
                Poll::Ready(())
            }
        },
        YieldState::Done => panic!("YieldTask polled after completion"),
    }
}

impl Future for YieldTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        poll_yield_state(&mut self.get_mut().state, context, |slot| {
            task(move || fire_yield_slot(&slot));
        })
    }
}

/// Yields once through a fresh `MessageChannel` task.
#[must_use]
pub fn yield_task() -> YieldTask {
    YieldTask {
        state: YieldState::Fresh,
    }
}

#[cfg(test)]
mod tests {
    use alloc::task::LocalWake;
    use core::task::{ContextBuilder, Waker};

    use super::*;

    struct CountWake {
        wakes: Cell<usize>,
    }

    impl CountWake {
        fn new() -> Rc<Self> {
            Rc::new(Self {
                wakes: Cell::new(0),
            })
        }

        fn count(&self) -> usize {
            self.wakes.get()
        }
    }

    impl LocalWake for CountWake {
        fn wake(self: Rc<Self>) {
            self.wakes.set(self.wakes.get() + 1);
        }

        fn wake_by_ref(self: &Rc<Self>) {
            self.wakes.set(self.wakes.get() + 1);
        }
    }

    fn with_local_context<R>(wake: &Rc<CountWake>, f: impl FnOnce(&mut Context<'_>) -> R) -> R {
        let local = LocalWaker::from(Rc::clone(wake));
        let mut context = ContextBuilder::from_waker(Waker::noop())
            .local_waker(&local)
            .build();
        f(&mut context)
    }

    #[test]
    fn yield_task_ignores_spurious_repolls_and_updates_waker() {
        let mut state = YieldState::Fresh;
        let scheduled = Cell::new(0usize);
        let mut captured_slot = None;

        let w1 = CountWake::new();
        let w2 = CountWake::new();
        let w3 = CountWake::new();

        // First poll schedules once and stays Pending.
        let poll1 = with_local_context(&w1, |cx| {
            poll_yield_state(&mut state, cx, |slot| {
                scheduled.set(scheduled.get() + 1);
                captured_slot = Some(slot);
            })
        });
        assert!(poll1.is_pending());
        assert_eq!(scheduled.get(), 1);

        // Spurious re-polls before the task fires stay Pending, do not re-schedule, and swap waker.
        let poll2 = with_local_context(&w2, |cx| {
            poll_yield_state(&mut state, cx, |_| unreachable!("must not reschedule"))
        });
        assert!(poll2.is_pending());

        let poll3 = with_local_context(&w3, |cx| {
            poll_yield_state(&mut state, cx, |_| unreachable!("must not reschedule"))
        });
        assert!(poll3.is_pending());
        assert_eq!(w1.count(), 0);
        assert_eq!(w2.count(), 0);
        assert_eq!(w3.count(), 0);

        // Firing the scheduled task wakes only the latest registered waker.
        let slot = captured_slot.expect("task must have been scheduled");
        fire_yield_slot(&slot);
        assert_eq!(w1.count(), 0);
        assert_eq!(w2.count(), 0);
        assert_eq!(w3.count(), 1);

        // Duplicate fires are ignored.
        fire_yield_slot(&slot);
        assert_eq!(w3.count(), 1);

        // Next poll resolves Ready(()).
        let poll4 = with_local_context(&w3, |cx| {
            poll_yield_state(&mut state, cx, |_| unreachable!("must not reschedule"))
        });
        assert_eq!(poll4, Poll::Ready(()));
    }

    #[test]
    #[should_panic(expected = "YieldTask polled after completion")]
    fn yield_task_panics_when_polled_after_ready() {
        let mut state = YieldState::Fresh;
        let wake = CountWake::new();
        let mut captured_slot = None;

        let _ = with_local_context(&wake, |cx| {
            poll_yield_state(&mut state, cx, |slot| captured_slot = Some(slot))
        });
        fire_yield_slot(&captured_slot.unwrap());

        assert_eq!(
            with_local_context(&wake, |cx| poll_yield_state(&mut state, cx, |_| {})),
            Poll::Ready(())
        );
        let _ = with_local_context(&wake, |cx| poll_yield_state(&mut state, cx, |_| {}));
    }

    #[test]
    fn promise_future_updates_waker_and_resolves_once() {
        let state = Rc::new(Cell::new(PromiseState::Fresh));
        let w1 = CountWake::new();
        let w2 = CountWake::new();
        let mut subscribed = None;

        let poll1 = with_local_context(&w1, |cx| {
            poll_promise_state(&state, cx, |shared| {
                subscribed = Some(shared);
                Ok(())
            })
        });
        assert!(poll1.is_pending());

        // Re-poll with a new waker before settlement replaces the stored waker.
        let poll2 = with_local_context(&w2, |cx| {
            poll_promise_state(&state, cx, |_| unreachable!("must not resubscribe"))
        });
        assert!(poll2.is_pending());

        let shared = subscribed.expect("subscription must run on first poll");
        settle_promise_state(&shared, Ok(JsValue::from_raw(42)));
        assert_eq!(w1.count(), 0);
        assert_eq!(w2.count(), 1);

        // A buggy thenable calling settlement a second time is ignored.
        settle_promise_state(&shared, Err(JsValue::from_raw(99)));
        assert_eq!(w2.count(), 1);

        let poll3 = with_local_context(&w2, |cx| {
            poll_promise_state(&state, cx, |_| unreachable!("must not resubscribe"))
        });
        match poll3 {
            Poll::Ready(Ok(value)) => assert_eq!(value.raw(), 42),
            _ => panic!("expected Poll::Ready(Ok(42))"),
        }
    }

    #[test]
    fn promise_future_delivers_rejection() {
        let state = Rc::new(Cell::new(PromiseState::Fresh));
        let wake = CountWake::new();
        let mut subscribed = None;

        assert!(with_local_context(&wake, |cx| {
            poll_promise_state(&state, cx, |shared| {
                subscribed = Some(shared);
                Ok(())
            })
        })
        .is_pending());

        settle_promise_state(&subscribed.unwrap(), Err(JsValue::from_raw(7)));
        assert_eq!(wake.count(), 1);

        let poll2 = with_local_context(&wake, |cx| {
            poll_promise_state(&state, cx, |_| unreachable!("must not resubscribe"))
        });
        match poll2 {
            Poll::Ready(Err(error)) => assert_eq!(error.raw(), 7),
            _ => panic!("expected Poll::Ready(Err(7))"),
        }
    }

    #[test]
    #[should_panic(expected = "PromiseFuture polled after completion")]
    fn promise_future_panics_when_polled_after_ready() {
        let state = Rc::new(Cell::new(PromiseState::Fresh));
        let wake = CountWake::new();
        let mut subscribed = None;

        let _ = with_local_context(&wake, |cx| {
            poll_promise_state(&state, cx, |shared| {
                subscribed = Some(shared);
                Ok(())
            })
        });
        settle_promise_state(&subscribed.unwrap(), Ok(JsValue::from_raw(1)));

        assert!(
            with_local_context(&wake, |cx| poll_promise_state(&state, cx, |_| Ok(()))).is_ready()
        );
        let _ = with_local_context(&wake, |cx| poll_promise_state(&state, cx, |_| Ok(())));
    }

    #[test]
    #[should_panic(expected = "PromiseFuture polled after completion")]
    fn promise_future_subscription_error_transitions_to_done() {
        let state = Rc::new(Cell::new(PromiseState::Fresh));
        let wake = CountWake::new();
        let mut subscribed = None;

        // Simulate a thenable that invokes settlement synchronously and then throws.
        let poll1 = with_local_context(&wake, |cx| {
            poll_promise_state(&state, cx, |shared| {
                settle_promise_state(&shared, Ok(JsValue::from_raw(10)));
                subscribed = Some(shared);
                Err(JsValue::from_raw(55))
            })
        });
        match poll1 {
            Poll::Ready(Err(error)) => assert_eq!(error.raw(), 55),
            _ => panic!("expected Poll::Ready(Err(55))"),
        }

        // Late callbacks after Done are ignored.
        settle_promise_state(&subscribed.unwrap(), Ok(JsValue::from_raw(11)));
        assert_eq!(wake.count(), 1);

        // Polling again after subscription error must panic as poll-after-completion.
        let _ = with_local_context(&wake, |cx| poll_promise_state(&state, cx, |_| Ok(())));
    }

    #[test]
    fn promise_has_same_layout_as_js_value() {
        assert_eq!(
            core::mem::size_of::<Promise>(),
            core::mem::size_of::<JsValue>()
        );
        assert_eq!(
            core::mem::align_of::<Promise>(),
            core::mem::align_of::<JsValue>()
        );
        fn assert_host_value<T: HostValue + Copy>() {}
        assert_host_value::<Promise>();
    }

    #[test]
    fn typed_promise_is_transparent_over_promise() {
        use alloc::string::String;

        assert_eq!(
            core::mem::size_of::<TypedPromise<()>>(),
            core::mem::size_of::<Promise>()
        );
        assert_eq!(
            core::mem::size_of::<TypedPromise<String>>(),
            core::mem::size_of::<Promise>()
        );
        assert_eq!(
            core::mem::align_of::<TypedPromise<Result<String, u64>>>(),
            core::mem::align_of::<Promise>()
        );

        let raw = Promise::from_raw(123);
        let typed: TypedPromise<String> = TypedPromise::from_promise(raw);
        let copied = typed;
        let cloned = Clone::clone(&copied);
        assert_eq!(typed.into_promise().raw(), 123);
        let as_promise: Promise = cloned.into();
        assert_eq!(as_promise.raw(), 123);
    }
}
