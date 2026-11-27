//! `rustc/tests/ui`-style tests for `async`/manual pollers, `Pin`, `Cell`, `RefCell`, `Rc`/`Weak`, `Sync` cells.

use std::cell::{Cell, OnceCell, RefCell};
use std::future::Future;
use std::pin::{Pin, pin};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::task::{Context, Poll, Waker};

struct YieldOnce {
    yielded: bool,
    value: i32,
}

impl Future for YieldOnce {
    type Output = i32;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<i32> {
        if self.yielded {
            Poll::Ready(self.value)
        } else {
            self.yielded = true;
            Poll::Pending
        }
    }
}

fn block_on_manual<F: Future>(fut: F) -> (F::Output, usize) {
    let mut fut = pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut polls = 0;
    loop {
        polls += 1;
        if let Poll::Ready(val) = fut.as_mut().poll(&mut cx) {
            return (val, polls);
        }
        assert!(polls < 100, "manual future did not converge");
    }
}

#[test]
fn ui_async_fn_multi_yield_manual_poller() {
    async fn compute(base: i32) -> String {
        let a = YieldOnce {
            yielded: false,
            value: base + 1,
        }
        .await;
        let b = YieldOnce {
            yielded: false,
            value: a * 3,
        }
        .await;
        format!("res={a}+{b}={}", a + b)
    }

    let (out, polls) = block_on_manual(compute(6));
    assert_eq!(out, "res=7+21=28");
    assert_eq!(polls, 3);
}

#[test]
fn ui_async_future_drop_while_suspended_runs_local_drops() {
    struct Guard(Rc<Cell<usize>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    let drops = Rc::new(Cell::new(0));
    {
        let drops_clone = Rc::clone(&drops);
        let fut = async move {
            let _g = Guard(drops_clone);
            YieldOnce {
                yielded: false,
                value: 1,
            }
            .await
        };
        let mut pinned = pin!(fut);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        assert!(matches!(pinned.as_mut().poll(&mut cx), Poll::Pending));
        assert_eq!(drops.get(), 0);
    }
    assert_eq!(drops.get(), 1);
}

#[test]
fn ui_cell_refcell_and_once_cell() {
    let c = Cell::new(10u32);
    assert_eq!(c.replace(25), 10);
    assert_eq!(c.get(), 25);
    c.update(|v| v + 5);
    assert_eq!(c.get(), 30);

    let rc = RefCell::new(vec![1, 2]);
    {
        let r1 = rc.borrow();
        let r2 = rc.borrow();
        assert_eq!(*r1, vec![1, 2]);
        assert_eq!(*r2, vec![1, 2]);
        assert!(rc.try_borrow_mut().is_err());
    }
    rc.borrow_mut().push(3);
    assert_eq!(*rc.borrow(), vec![1, 2, 3]);

    let once = OnceCell::new();
    assert!(once.get().is_none());
    assert_eq!(*once.get_or_init(|| String::from("init")), "init");
    assert_eq!(*once.get_or_init(|| String::from("other")), "init");
}

#[test]
#[should_panic]
fn ui_refcell_double_mut_borrow_panics() {
    let cell = RefCell::new(0);
    let _a = cell.borrow_mut();
    let _b = cell.borrow_mut();
}

#[test]
fn ui_rc_weak_upgrade_and_drop() {
    let strong = Rc::new(String::from("shared-node"));
    let weak: Weak<String> = Rc::downgrade(&strong);
    assert_eq!(Rc::strong_count(&strong), 1);
    assert_eq!(Rc::weak_count(&strong), 1);

    let upgraded = weak.upgrade().expect("alive");
    assert_eq!(&*upgraded, "shared-node");
    assert_eq!(Rc::strong_count(&strong), 2);

    drop(upgraded);
    drop(strong);
    assert!(weak.upgrade().is_none());
}

#[test]
fn ui_arc_mutex_rwlock_and_oncelock() {
    let shared = Arc::new(Mutex::new(vec![10, 20]));
    {
        let s2 = Arc::clone(&shared);
        s2.lock().unwrap().push(30);
    }
    assert_eq!(*shared.lock().unwrap(), vec![10, 20, 30]);

    let rw = RwLock::new(String::from("v1"));
    assert_eq!(&*rw.read().unwrap(), "v1");
    *rw.write().unwrap() = String::from("v2");
    assert_eq!(&*rw.read().unwrap(), "v2");

    static GLOBAL_CELL: OnceLock<u32> = OnceLock::new();
    assert_eq!(*GLOBAL_CELL.get_or_init(|| 1234), 1234);
    assert_eq!(GLOBAL_CELL.get(), Some(&1234));
}
