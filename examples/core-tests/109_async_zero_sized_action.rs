//! Noncapturing actions survive suspension as zero-sized coroutine fields.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Poll, Waker};

struct PendOnce(bool);

impl Future for PendOnce {
    type Output = i32;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<i32> {
        if self.0 {
            Poll::Ready(7)
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        print_i32(90);
    }
}

#[inline(never)]
async fn with_action(action: impl FnOnce(i32) -> Result<i32, i32>) -> Result<i32, i32> {
    let guard = Guard;
    print_i32(10);
    let input = PendOnce(false).await;
    print_i32(11);
    let result = action(input);
    print_i32(30);
    drop(guard);
    let value = result?;
    Ok(value + PendOnce(false).await)
}

fn drive(future: impl Future<Output = Result<i32, i32>>) {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Pending => print_i32(40),
            Poll::Ready(result) => {
                print_i32(result.unwrap_or_else(|error| -error));
                break;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    drive(with_action(|value| {
        print_i32(20);
        Ok(value + 1)
    }));
    drive(with_action(|value| {
        print_i32(21);
        Err(value)
    }));
    let captured = 3;
    drive(with_action(move |value| {
        print_i32(22);
        Ok(value + captured)
    }));
    // Cancellation after the first suspension drops initialized state without running action.
    {
        let mut future = pin!(with_action(|_| {
            print_i32(99);
            Ok(0)
        }));
        let mut context = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut context).is_pending());
        print_i32(50);
    }
    print_i32(60);
}
