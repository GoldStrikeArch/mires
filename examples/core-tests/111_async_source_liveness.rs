#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::future::Future;
use core::pin::{pin, Pin};
use core::task::{Context, Poll, Waker};

struct PendOnce(bool);

impl Future for PendOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

struct Guard(i32);

impl Drop for Guard {
    fn drop(&mut self) {
        print_i32(self.0);
    }
}

async fn borrowed(x: i32) -> i32 {
    let retained = [x, x + 1];
    let view = &retained;
    let dead = x + 100;
    print_i32(dead);
    PendOnce(false).await;
    view[1]
}

async fn moved_guard(x: i32) -> i32 {
    let moved = Guard(x);
    drop(moved);
    PendOnce(false).await;
    x + 1
}

async fn held_guard(x: i32) -> i32 {
    let held = Guard(x);
    PendOnce(false).await;
    let result = held.0 + 1;
    drop(held);
    result
}

async fn two_states(x: i32) -> i32 {
    let first = x + 1;
    PendOnce(false).await;
    let second = x + 2;
    print_i32(first);
    PendOnce(false).await;
    second
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
    }
}

#[no_mangle]
pub fn rust_entry() {
    print_i32(block_on(borrowed(4)));
    print_i32(block_on(moved_guard(6)));
    print_i32(block_on(held_guard(8)));
    print_i32(block_on(two_states(10)));
}
