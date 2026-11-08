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

#[inline(never)]
fn checkpoint<const N: i32>() {
    print_i32(N);
}

#[inline(never)]
fn take(value: Guard) -> Guard {
    value
}

async fn initialization(flag: bool) {
    let early: Guard;
    checkpoint::<10>();
    early = Guard(80);
    checkpoint::<11>();
    let pair = (Guard(90), Guard(91));
    checkpoint::<12>();
    drop(pair.0);
    checkpoint::<13>();
    let conditional: Guard;
    if flag { conditional = Guard(92); }
    checkpoint::<14>();
    PendOnce(false).await;
    checkpoint::<15>();
    let returned = take(early);
    checkpoint::<16>();
    drop(pair.1);
    drop(returned);
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
    block_on(initialization(true));
    block_on(initialization(false));
}
