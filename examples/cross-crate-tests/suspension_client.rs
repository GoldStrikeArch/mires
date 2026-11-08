#![no_std]
#![no_main]

#[path = "../core-tests/prelude.rs"]
mod prelude;

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};

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
    prelude::print_i32(block_on(suspension_provider::relay(7)));
    prelude::print_str(block_on(suspension_provider::relay("generic")));
    prelude::print_i32(block_on(suspension_provider::repeated(9, 3)));
    prelude::print_i32(block_on(suspension_provider::immediate(11)));
}
