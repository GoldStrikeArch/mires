//! RawWakerVTable in a static, built from closures that USE their data pointer.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

struct Counts { clones: Cell<u32>, wakes: Cell<u32>, wake_refs: Cell<u32>, drops: Cell<u32> }

static VT: RawWakerVTable = RawWakerVTable::new(
    |p| { let c = unsafe { &*(p as *const Counts) }; c.clones.set(c.clones.get() + 1); RawWaker::new(p, &VT) },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.wakes.set(c.wakes.get() + 1); },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.wake_refs.set(c.wake_refs.get() + 1); },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.drops.set(c.drops.get() + 1); },
);

struct YieldOnce(bool);
impl Future for YieldOnce {
    type Output = u32;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
        if self.0 { Poll::Ready(7) } else { self.0 = true; cx.waker().wake_by_ref(); let w = cx.waker().clone(); w.wake(); Poll::Pending }
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let c = Counts { clones: Cell::new(0), wakes: Cell::new(0), wake_refs: Cell::new(0), drops: Cell::new(0) };
    let waker = unsafe { Waker::from_raw(RawWaker::new(&c as *const Counts as *const (), &VT)) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = YieldOnce(false);
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
    let mut polls = 0;
    loop {
        polls += 1;
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) { print_u32(v); break; }
    }
    print_u32(polls);
    let w2 = waker.clone();
    print_bool(w2.will_wake(&waker));
    print_bool(VT == VT);
    print_bool(*w2.vtable() == *waker.vtable());
    w2.wake_by_ref();
    drop(w2);
    drop(waker);
    print_u32(c.clones.get()); print_u32(c.wakes.get()); print_u32(c.wake_refs.get()); print_u32(c.drops.get());
}
