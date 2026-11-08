#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use mires_js::{js_bindings, js_export, JsValue};

js_bindings! {
    #[js(call = "cleanupNote")]
    fn note(phase: f64);
}

struct Probe {
    polled: bool,
    reject: bool,
    value: JsValue,
}

impl Future for Probe {
    type Output = Result<JsValue, JsValue>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        context.waker().wake_by_ref();
        context.waker().wake_by_ref();
        if self.polled {
            note(20.0);
            Poll::Ready(if self.reject {
                Err(self.value)
            } else {
                Ok(self.value)
            })
        } else {
            self.polled = true;
            note(10.0);
            Poll::Pending
        }
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        note(80.0);
    }
}

#[js_export]
pub fn start(value: JsValue, reject: bool, native: bool) -> JsValue {
    let future = Probe {
        polled: false,
        reject,
        value,
    };
    if native {
        mires_js::future_to_native_promise(future)
    } else {
        mires_js::future_to_promise(future)
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
