#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use js_extern_macro::{js_bindings, js_export};

js_bindings! {
    #[js(call = "taskCleanupNote")]
    fn note(phase: f64);
}

struct Probe {
    polled: bool,
}

impl Future for Probe {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        context.waker().wake_by_ref();
        context.waker().wake_by_ref();
        if self.polled {
            note(20.0);
            Poll::Ready(())
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
pub fn start(owned: bool) {
    let future = Probe { polled: false };
    if owned {
        view_async::spawn(future);
    } else {
        mires_js::spawn_local(future);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
