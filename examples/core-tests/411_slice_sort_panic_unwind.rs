//! MIRES-022: throwing/panicking inside `slice::sort_by` and `slice::sort_unstable_by` comparator
//! must preserve exact 1x Drop per element (zero duplication, zero loss) AND preserve interior
//! mutability (`Cell` / `RefCell`) writes performed during comparisons before the throw.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use mires_js::{throw_value, try_host, JsValue};
use prelude::{print_bool, print_usize};

struct Tracked {
    id: u32,
    seen: Cell<u32>,
    drops: Rc<Cell<usize>>,
    drop_xor: Rc<Cell<u32>>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
        self.drop_xor.set(self.drop_xor.get() ^ (self.id + 1));
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Stable sort_by: throw mid-sort, verify interior mutability & exact 1x drop
    let drops = Rc::new(Cell::new(0usize));
    let drop_xor = Rc::new(Cell::new(0u32));
    let expected_xor: u32 = (0..12u32).fold(0, |acc, id| acc ^ (id + 1));

    let threw_stable = {
        let items: RefCell<Vec<Tracked>> = RefCell::new(
            (0..12u32)
                .rev()
                .map(|id| Tracked {
                    id,
                    seen: Cell::new(0),
                    drops: Rc::clone(&drops),
                    drop_xor: Rc::clone(&drop_xor),
                })
                .collect(),
        );
        let cmps = Cell::new(0usize);
        let res = try_host(|| {
            items.borrow_mut().sort_by(|a, b| {
                a.seen.set(a.seen.get() + 1);
                b.seen.set(b.seen.get() + 1);
                let c = cmps.get() + 1;
                cmps.set(c);
                if c == 7 {
                    throw_value(JsValue::erase("sort-boom"));
                }
                a.id.cmp(&b.id)
            });
            JsValue::undefined()
        });
        let mut items = items.into_inner();
        let total_seen: u32 = items.iter().map(|it| it.seen.get()).sum();
        // 7 comparisons touched 2 elements each = 14 increments, all preserved in place!
        print_usize(total_seen as usize);
        // Every element still present before drop, re-sort succeeds cleanly
        items.sort_by(|a, b| a.id.cmp(&b.id));
        print_bool(items.iter().enumerate().all(|(i, it)| it.id == i as u32));
        res.is_err()
    };
    print_bool(threw_stable);
    print_usize(drops.get());
    print_bool(drop_xor.get() == expected_xor);

    // 2. Unstable sort_unstable_by: throw mid-sort across a local Vec inside try_host (unwind drops!)
    let drops2 = Rc::new(Cell::new(0usize));
    let drop_xor2 = Rc::new(Cell::new(0u32));
    let d2 = Rc::clone(&drops2);
    let x2 = Rc::clone(&drop_xor2);
    let threw_unstable = try_host(move || {
        let mut items: Vec<Tracked> = (0..12u32)
            .rev()
            .map(|id| Tracked {
                id,
                seen: Cell::new(0),
                drops: Rc::clone(&d2),
                drop_xor: Rc::clone(&x2),
            })
            .collect();
        let mut cmps = 0usize;
        items.sort_unstable_by(|a, b| {
            cmps += 1;
            if cmps == 5 {
                throw_value(JsValue::erase("unstable-boom"));
            }
            a.id.cmp(&b.id)
        });
        JsValue::undefined()
    })
    .is_err();
    print_bool(threw_unstable);
    if cfg!(panic = "unwind") {
        print_bool(drops2.get() == 12 && drop_xor2.get() == expected_xor);
    } else {
        print_bool(true);
    }

    // 3. Direct scalar Cell<i32> interior mutability during throwing sort
    let cells: RefCell<Vec<Cell<i32>>> = RefCell::new((0..8).rev().map(Cell::new).collect());
    let n = Cell::new(0usize);
    let _ = try_host(|| {
        cells.borrow_mut().sort_by(|a, b| {
            a.set(a.get() + 100);
            let c = n.get() + 1;
            n.set(c);
            if c == 3 {
                throw_value(JsValue::erase("cell-boom"));
            }
            a.get().cmp(&b.get())
        });
        JsValue::undefined()
    });
    let cells = cells.into_inner();
    let sum: i32 = cells.iter().map(Cell::get).sum();
    // Original sum = 0+..+7 = 28; 3 calls incremented `a` by 100 each => 328
    print_bool(sum == 328);
}
