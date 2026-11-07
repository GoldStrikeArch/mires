//! Three wakes before the scheduled poll runs cost ONE poll.
//!
//! Without the flag, a future woken in a loop queues a microtask per wake and the queue grows
//! without bound; the poll count is the only thing that shows it, which is why this fixture counts
//! polls rather than asserting a shape. The second poll is the one the wakes bought, and there is
//! exactly one however many times the waker was called.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::say;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Pends once, waking itself three times on the way out.
struct WakeThrice {
    pended: bool,
}

impl Future for WakeThrice {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        say("poll");
        if self.pended {
            return Poll::Ready(());
        }
        self.pended = true;
        let waker = context.waker();
        waker.wake_by_ref();
        waker.wake_by_ref();
        waker.wake_by_ref();
        Poll::Pending
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn run() {
    view_async::spawn(WakeThrice { pended: false });
}
