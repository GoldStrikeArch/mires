//! RawWakerVTable built from closures (as core's Waker::noop does), with closures that USE their
//! data pointer, and identity of the noop waker.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::future::Future;
use core::hint::black_box;
use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

struct Counts { clones: Cell<u32>, wakes: Cell<u32>, wake_refs: Cell<u32>, drops: Cell<u32> }

// closures in a static vtable that use the data pointer
static VT: RawWakerVTable = RawWakerVTable::new(
    |p| { let c = unsafe { &*(p as *const Counts) }; c.clones.set(c.clones.get() + 1); RawWaker::new(p, &VT) },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.wakes.set(c.wakes.get() + 1); },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.wake_refs.set(c.wake_refs.get() + 1); },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.drops.set(c.drops.get() + 1); },
);
// the same in a const (promoted) vtable
const CVT: &RawWakerVTable = &RawWakerVTable::new(
    |p| { let c = unsafe { &*(p as *const Counts) }; c.clones.set(c.clones.get() + 10); RawWaker::new(p, CVT) },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.wakes.set(c.wakes.get() + 10); },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.wake_refs.set(c.wake_refs.get() + 10); },
    |p| { let c = unsafe { &*(p as *const Counts) }; c.drops.set(c.drops.get() + 10); },
);

fn report(c: &Counts) {
    print_u32(c.clones.get()); print_u32(c.wakes.get()); print_u32(c.wake_refs.get()); print_u32(c.drops.get());
}

struct YieldOnce(bool);
impl Future for YieldOnce {
    type Output = u32;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
        if self.0 { Poll::Ready(7) } else { self.0 = true; cx.waker().wake_by_ref(); let w = cx.waker().clone(); w.wake(); Poll::Pending }
    }
}

fn drive(vt: &'static RawWakerVTable, c: &Counts) {
    let waker = unsafe { Waker::from_raw(RawWaker::new(c as *const Counts as *const (), vt)) };
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
    w2.wake_by_ref();
    drop(w2);
    drop(waker);
    report(c);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let c = Counts { clones: Cell::new(0), wakes: Cell::new(0), wake_refs: Cell::new(0), drops: Cell::new(0) };
    drive(&VT, &c);
    let c2 = Counts { clones: Cell::new(0), wakes: Cell::new(0), wake_refs: Cell::new(0), drops: Cell::new(0) };
    drive(CVT, &c2);

    // the noop waker
    let n = Waker::noop();
    let m = Waker::noop();
    print_bool(n.will_wake(m));
    print_bool(n.data() == m.data());
    print_bool(core::ptr::eq(n.vtable(), m.vtable()));
    print_bool(*n.vtable() == *m.vtable());
    print_bool(black_box(n).vtable() == black_box(Waker::noop()).vtable());
    let k = n.clone();
    print_bool(k.will_wake(n));
    print_bool(*k.vtable() == *n.vtable());
    k.wake_by_ref();
    k.wake();
    let mut cx = Context::from_waker(Waker::noop());
    let mut fut = YieldOnce(false);
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
    print_bool(fut.as_mut().poll(&mut cx).is_pending());
    print_bool(fut.as_mut().poll(&mut cx).is_ready());
    // vtable struct equality of user vtables
    print_bool(VT == VT);
    print_bool(*CVT == *CVT);
    print_bool(VT == *CVT);
}
