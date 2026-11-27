//! `rustc/tests/ui`-style tests for Drop ordering, unwinding, `catch_unwind`, and `panic_any`.

use std::cell::RefCell;
use std::mem::{self, ManuallyDrop};
use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};
use std::rc::Rc;

struct DropLog {
    id: u32,
    log: Rc<RefCell<Vec<u32>>>,
}

impl Drop for DropLog {
    fn drop(&mut self) {
        self.log.borrow_mut().push(self.id);
    }
}

#[test]
fn ui_drop_lifo_order_in_blocks_and_structs() {
    let log = Rc::new(RefCell::new(Vec::new()));
    {
        let _a = DropLog {
            id: 1,
            log: Rc::clone(&log),
        };
        let _b = DropLog {
            id: 2,
            log: Rc::clone(&log),
        };
        {
            let _c = DropLog {
                id: 3,
                log: Rc::clone(&log),
            };
        }
        assert_eq!(*log.borrow(), vec![3]);
    }
    assert_eq!(*log.borrow(), vec![3, 2, 1]);
}

#[test]
fn ui_drop_struct_field_declaration_order() {
    struct Holder {
        _first: DropLog,
        _second: DropLog,
        _third: DropLog,
    }
    let log = Rc::new(RefCell::new(Vec::new()));
    {
        let _h = Holder {
            _first: DropLog {
                id: 10,
                log: Rc::clone(&log),
            },
            _second: DropLog {
                id: 20,
                log: Rc::clone(&log),
            },
            _third: DropLog {
                id: 30,
                log: Rc::clone(&log),
            },
        };
    }
    assert_eq!(*log.borrow(), vec![10, 20, 30]);
}

#[test]
fn ui_drop_runs_during_unwind_and_panicking_flag() {
    struct UnwindObserver {
        id: u32,
        events: Rc<RefCell<Vec<(u32, bool)>>>,
    }
    impl Drop for UnwindObserver {
        fn drop(&mut self) {
            self.events
                .borrow_mut()
                .push((self.id, std::thread::panicking()));
        }
    }

    let events = Rc::new(RefCell::new(Vec::new()));
    let events_inner = Rc::clone(&events);

    let res = catch_unwind(AssertUnwindSafe(move || {
        let _outer = UnwindObserver {
            id: 1,
            events: Rc::clone(&events_inner),
        };
        let _inner = UnwindObserver {
            id: 2,
            events: Rc::clone(&events_inner),
        };
        panic!("boom-unwind");
    }));

    assert!(res.is_err());
    assert!(!std::thread::panicking());
    assert_eq!(*events.borrow(), vec![(2, true), (1, true)]);
}

#[test]
fn ui_nested_catch_unwind_and_custom_panic_any() {
    #[derive(Debug, PartialEq, Eq)]
    struct CustomPayload {
        code: u32,
        tag: String,
    }

    let outer = catch_unwind(|| {
        let inner = catch_unwind(|| {
            panic_any(CustomPayload {
                code: 418,
                tag: String::from("teapot"),
            });
        });
        let payload = inner.expect_err("inner should catch custom payload");
        let custom = payload
            .downcast_ref::<CustomPayload>()
            .expect("exact CustomPayload type");
        assert_eq!(custom.code, 418);
        assert_eq!(custom.tag, "teapot");
        "recovered"
    });

    assert_eq!(outer.unwrap(), "recovered");
}

#[test]
fn ui_partial_move_drop_and_manually_drop() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let pair = (
        DropLog {
            id: 1,
            log: Rc::clone(&log),
        },
        DropLog {
            id: 2,
            log: Rc::clone(&log),
        },
    );
    let moved_first = pair.0;
    // Only pair.1 is dropped at end of block; moved_first lives longer.
    let mut manual = ManuallyDrop::new(DropLog {
        id: 99,
        log: Rc::clone(&log),
    });
    assert_eq!(moved_first.id, 1);
    drop(moved_first);
    assert_eq!(*log.borrow(), vec![1]);
    // SAFETY: `manual` is never accessed again after `ManuallyDrop::drop`.
    unsafe { ManuallyDrop::drop(&mut manual) };
    assert_eq!(*log.borrow(), vec![1, 99]);
    drop(pair.1);
    assert_eq!(*log.borrow(), vec![1, 99, 2]);
}

#[test]
fn ui_mem_replace_swap_take() {
    let mut a = String::from("alpha");
    let mut b = String::from("beta");
    mem::swap(&mut a, &mut b);
    assert_eq!(a, "beta");
    assert_eq!(b, "alpha");

    let prev = mem::replace(&mut a, String::from("gamma"));
    assert_eq!(prev, "beta");
    assert_eq!(a, "gamma");

    let taken = mem::take(&mut a);
    assert_eq!(taken, "gamma");
    assert!(a.is_empty());
}

#[test]
#[should_panic(expected = "expected-substring-77")]
fn ui_should_panic_expected_formatted() {
    let code = 77;
    panic!("prefix expected-substring-{code} suffix");
}

#[test]
#[should_panic]
fn ui_should_panic_from_core_bounds() {
    let arr = [10, 20, 30];
    let idx = std::hint::black_box(5);
    let _ = arr[idx];
}
