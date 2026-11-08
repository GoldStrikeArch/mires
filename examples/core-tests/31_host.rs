//! The safe dynamic host boundary and its standalone local future executor.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::rc::Rc;
use core::cell::Cell;
use core::future::Future;
use core::marker::PhantomPinned;
use core::pin::Pin;
use core::task::{Context, Poll};
use mires_js::{
    await_promise, callback0, callback1, callback2, construct, future_to_promise, invoke,
    js_bindings, js_export, throw_value, try_host, yield_task, JsValue,
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

js_bindings! {
    #[js(call = "console.log")]
    fn log(label: &str, value: JsValue);

    #[js(call = "queueMicrotask")]
    fn queue_microtask(callback: JsValue);

    #[js(global, get = "Object")]
    fn object_constructor() -> JsValue;

    #[js(global, get = "Array")]
    fn array_constructor() -> JsValue;

    #[js(global, get = "Array.prototype.push")]
    fn array_push() -> JsValue;

    #[js(global, get = "Error")]
    fn error_constructor() -> JsValue;

    #[js(set = "then")]
    fn set_then(value: JsValue, callback: JsValue);

    #[js(method = "then")]
    fn promise_then(value: JsValue, resolved: JsValue, rejected: JsValue) -> JsValue;
}

fn report(label: &str, value: bool) {
    log(label, JsValue::erase(value));
}

fn thenable(value: JsValue, succeed: bool) -> JsValue {
    let object = construct(object_constructor(), &[]);
    let settle = callback2(move |resolve, reject| {
        let function = if succeed { resolve } else { reject };
        let _ = invoke(function, JsValue::undefined(), &[value]);
        JsValue::undefined()
    });
    set_then(object, settle);
    object
}

/// A custom `!Unpin` future whose own address must stay fixed across polls.
struct PinnedFuture {
    first_address: Cell<usize>,
    pended: Cell<bool>,
    _marker: PhantomPinned,
}

impl Future for PinnedFuture {
    type Output = Result<JsValue, JsValue>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let me = self.as_ref().get_ref();
        let address = core::ptr::from_ref(me) as usize;
        if !me.pended.replace(true) {
            me.first_address.set(address);
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        report("stable-pin", me.first_address.get() == address);
        Poll::Ready(Ok(JsValue::undefined()))
    }
}

/// A ready future whose destructor must run before its Promise rejection is observed.
struct DropReady {
    dropped: Rc<Cell<bool>>,
    reason: JsValue,
}

impl Future for DropReady {
    type Output = Result<JsValue, JsValue>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Err(self.reason))
    }
}

impl Drop for DropReady {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}

/// A closure capture whose destructor makes one-shot host cleanup observable.
struct TryCapture(Rc<Cell<bool>>);

impl Drop for TryCapture {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[js_export]
pub fn rust_entry() {
    let zero = callback0(|| JsValue::erase("zero"));
    report(
        "callback0",
        invoke(zero, JsValue::undefined(), &[]).strictly_equals(JsValue::erase("zero")),
    );

    let one = callback1(|value| value);
    report(
        "callback1",
        invoke(one, JsValue::undefined(), &[JsValue::erase("one")])
            .strictly_equals(JsValue::erase("one")),
    );

    let two = callback2(|first, second| {
        if first.strictly_equals(JsValue::erase("first")) {
            second
        } else {
            first
        }
    });
    report(
        "callback2",
        invoke(
            two,
            JsValue::undefined(),
            &[JsValue::erase("first"), JsValue::erase("second")],
        )
        .strictly_equals(JsValue::erase("second")),
    );

    let array = construct(array_constructor(), &[]);
    let length = invoke(array_push(), array, &[JsValue::erase("item")]).read::<u32>();
    report("this", length == 1);
    report(
        "construct",
        construct(error_constructor(), &[JsValue::erase("host")]).is_error(),
    );

    let thrown = construct(error_constructor(), &[JsValue::erase("thrown")]);
    let dropped = Rc::new(Cell::new(false));
    let capture = TryCapture(Rc::clone(&dropped));
    let caught = match try_host(move || {
        let _ = &capture;
        throw_value(thrown)
    }) {
        Ok(_) => false,
        Err(error) => error.strictly_equals(thrown),
    };
    report("throw", caught && dropped.get());

    let dropped = Rc::new(Cell::new(false));
    let capture = TryCapture(Rc::clone(&dropped));
    let returned = try_host(move || {
        let _ = &capture;
        JsValue::undefined()
    });
    report("try-drop", returned.is_ok() && dropped.get());
    report(
        "values",
        JsValue::undefined().is_undefined()
            && JsValue::null().is_null()
            && JsValue::erase(true).is_boolean()
            && JsValue::erase(1u32).is_number()
            && JsValue::erase("text").is_string()
            && JsValue::erase("text").is_truthy(),
    );

    let retained = JsValue::erase("retained");
    queue_microtask(callback0(move || {
        report(
            "retained",
            retained.strictly_equals(JsValue::erase("retained")),
        );
        JsValue::undefined()
    }));

    let _ = future_to_promise(async {
        log("side-start", JsValue::erase(true));
        let value = await_promise(thenable(JsValue::erase("side"), true)).await?;
        report("side-resume", value.strictly_equals(JsValue::erase("side")));
        Ok(value)
    });

    let _ = future_to_promise(async {
        log("future-start", JsValue::erase(true));
        let value = await_promise(thenable(JsValue::erase("resolved"), true)).await?;
        report("resolve", value.strictly_equals(JsValue::erase("resolved")));

        let rejected = await_promise(thenable(JsValue::erase("rejected"), false)).await;
        report(
            "reject",
            matches!(rejected, Err(value) if value.strictly_equals(JsValue::erase("rejected"))),
        );

        log("before-yield", JsValue::erase(true));
        yield_task().await;
        log("after-yield", JsValue::erase(true));
        Ok(JsValue::undefined())
    });

    let _ = future_to_promise(PinnedFuture {
        first_address: Cell::new(0),
        pended: Cell::new(false),
        _marker: PhantomPinned,
    });

    let dropped = Rc::new(Cell::new(false));
    let reason = construct(error_constructor(), &[JsValue::erase("drop-ready")]);
    let promise = future_to_promise(DropReady {
        dropped: Rc::clone(&dropped),
        reason,
    });
    let rejected = callback1(move |value| {
        report(
            "drop-before-reject",
            dropped.get() && value.strictly_equals(reason),
        );
        JsValue::undefined()
    });
    let _ = promise_then(promise, callback1(|_| JsValue::undefined()), rejected);

    log("entry-return", JsValue::erase(true));
}
