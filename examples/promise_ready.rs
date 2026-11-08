#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::future::Future;
use mires_js::{JsValue, js_bindings, js_export};

js_bindings! {
    #[js(call = "readyNote")]
    fn note(phase: f64, value: JsValue);
}

#[cfg(any(local_drop, captured_drop, helper_drop))]
struct Guard;

#[cfg(any(local_drop, captured_drop, helper_drop))]
impl Drop for Guard {
    fn drop(&mut self) {
        note(90.0, JsValue::undefined());
    }
}

#[cfg(helper_drop)]
#[inline(always)]
fn helper(value: JsValue) {
    let _guard = Guard;
    note(11.0, value);
}

#[cfg(inlined_helper)]
#[inline(always)]
fn identity(value: JsValue) -> JsValue {
    value
}

fn task(value: JsValue, reject: bool) -> impl Future<Output = Result<JsValue, JsValue>> {
    #[cfg(captured_drop)]
    let captured = Guard;
    #[cfg(borrowed)]
    let borrowed: &'static u32 = if reject { &17 } else { &19 };
    async move {
        #[cfg(inlined_helper)]
        let value = identity(value);
        #[cfg(local_drop)]
        let _guard = Guard;
        #[cfg(captured_drop)]
        let _guard = captured;
        #[cfg(helper_drop)]
        helper(value);
        #[cfg(borrowed)]
        if *borrowed == 17 {
            note(12.0, value);
        }
        #[cfg(suspending)]
        core::future::ready(()).await;
        note(10.0, value);
        if reject { Err(value) } else { Ok(value) }
    }
}

#[cfg(custom)]
struct Immediate(JsValue, bool);

#[cfg(custom)]
impl Future for Immediate {
    type Output = Result<JsValue, JsValue>;

    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        cx.waker().wake_by_ref();
        note(10.0, self.0);
        core::task::Poll::Ready(if self.1 { Err(self.0) } else { Ok(self.0) })
    }
}

#[js_export]
pub fn start(value: JsValue, reject: bool) -> JsValue {
    note(1.0, value);
    #[cfg(mixed)]
    if reject {
        let promise = mires_js::future_to_promise(async move {
            core::future::ready(()).await;
            note(10.0, value);
            Err(value)
        });
        note(2.0, value);
        return promise;
    }
    #[cfg(custom)]
    let future = Immediate(value, reject);
    #[cfg(not(custom))]
    let future = task(value, reject);
    let promise = mires_js::future_to_promise(future);
    note(2.0, value);
    promise
}

#[js_export]
pub fn inert(value: JsValue) {
    let future = task(value, false);
    note(0.0, value);
    drop(future);
    note(3.0, value);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
