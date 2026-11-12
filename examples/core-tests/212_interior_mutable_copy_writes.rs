//! A store through a shared view of a fresh value lands in that value, never in what it was built
//! or copied from.
//!
//! `Cell::set`, `RefCell::borrow_mut`, an atomic store and `UnsafeCell::get` write through a `&`,
//! and `UnsafeCell::raw_get` through a `*const`, so a shared view designates storage exactly as a
//! `&mut` does. The fusion planner kept a value declared only for a mutable view of it, and spelled
//! `let q = C { a: Cell::new(n), b: m }` at its one use, the shared borrow `&q.a`: once MIR inlining
//! (`JSC_RUST_MIR_INLINE=on`, as the package builders run) put the store into the same body, the
//! store's place folded through the literal to its operand and `q.a.set(99)` became `n = 99`, and
//! with a slice element or a static as the operand it overwrote the caller's array or the static.
//! The JIR lane otherwise declined the borrow, and the baseline text it fell back to made the same
//! write.
//!
//! Each case builds a value with an interior-mutable field out of a parameter, a slice element, a
//! static element, a local or a tuple field, stores through a shared view of it, and prints the
//! operand afterwards. `from_mut` turns a dead copy's `&mut` field into a `Cell` and sets it, and
//! `drop` writes a copy held in a struct from its `Drop` impl. `read_back` and `only_read` are the
//! controls: the stored value is read back, or the copy is only read through its shared view.
//!
//! On the base linker under `js-profile=release-balanced` with `JSC_RUST_OPT_LEVEL=z
//! JSC_RUST_MIR_INLINE=on`, the `Cell::set` and `UnsafeCell` cases from `parameters` to `loop` and
//! `unsafecell get`/`raw_get` are wrong, and `from_mut` is wrong without those settings as well;
//! `replace`, `whole struct`, `refcell`, `atomic`, `as_ptr` and `drop` are neighbours that were
//! already right. The expectations come from this file run natively at `-O0` and `-O3`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::{Cell, RefCell, UnsafeCell};
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

struct C {
    a: Cell<u32>,
    b: u32,
}

struct U {
    a: UnsafeCell<u32>,
    b: u32,
}

struct CP {
    p: Cell<P>,
    k: u32,
}

static NUMS: [u32; 3] = [10, 11, 12];
static PS: [P; 3] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }];

fn sum(s: &[P]) -> u32 {
    let mut t = 0;
    for p in s {
        t = t * 7 + p.x * 3 + p.y;
    }
    t
}

#[inline(never)]
fn parameters(n: u32, m: u32) -> u32 {
    let q = C { a: Cell::new(n), b: m };
    q.a.set(99);
    n * 1000 + m
}

#[inline(never)]
fn element(s: &[u32]) -> u32 {
    let q = C {
        a: Cell::new(s[black_box(1)]),
        b: 7,
    };
    q.a.set(99);
    s[1]
}

#[inline(never)]
fn static_element() -> u32 {
    let q = C {
        a: Cell::new(NUMS[black_box(1)]),
        b: 7,
    };
    q.a.set(99);
    NUMS[1]
}

#[inline(never)]
fn local(k: u32) -> u32 {
    let n = k + 1;
    let q = C { a: Cell::new(n), b: 7 };
    q.a.set(99);
    n
}

#[inline(never)]
fn tuple_field(h: &mut (u32, u32)) -> u32 {
    let q = C {
        a: Cell::new(h.0),
        b: 7,
    };
    q.a.set(99);
    h.0
}

#[inline(never)]
fn replace(n: u32, m: u32) -> u32 {
    let q = C { a: Cell::new(n), b: m };
    let old = q.a.replace(99);
    n * 1000 + m + old * 1_000_000
}

#[inline(never)]
fn in_a_loop(s: &[u32]) -> u32 {
    for i in 0..s.len() {
        let q = C {
            a: Cell::new(s[i]),
            b: 7,
        };
        q.a.set(99);
    }
    s[0] * 10000 + s[1] * 100 + s[2]
}

#[inline(never)]
fn whole_struct(s: &[P]) -> u32 {
    let q = CP {
        p: Cell::new(s[black_box(1)]),
        k: 1,
    };
    q.p.set(P { x: 99, y: 98 });
    sum(s) * 100 + q.k
}

#[inline(never)]
fn refcell(n: u32, m: u32) -> u32 {
    let q = RefCell::new(P { x: n, y: m });
    q.borrow_mut().y = 99;
    n * 1000 + m
}

#[inline(never)]
fn atomic(n: u32, m: u32) -> u32 {
    let q = (AtomicU32::new(n), m);
    q.0.store(99, Ordering::Relaxed);
    n * 1000 + m
}

#[inline(never)]
fn unsafecell_get(n: u32, m: u32) -> u32 {
    let q = U {
        a: UnsafeCell::new(n),
        b: m,
    };
    unsafe {
        *q.a.get() = 99;
    }
    n * 1000 + m
}

#[inline(never)]
fn unsafecell_raw_get(s: &[u32]) -> u32 {
    let q = U {
        a: UnsafeCell::new(s[black_box(1)]),
        b: 7,
    };
    let p = UnsafeCell::raw_get(&raw const q.a);
    unsafe {
        *p = 99;
    }
    s[1]
}

#[inline(never)]
fn as_ptr(n: u32, m: u32) -> u32 {
    let q = (Cell::new(n), m);
    unsafe {
        *q.0.as_ptr() = 99;
    }
    n * 1000 + m
}

#[inline(never)]
fn from_mut(s: &[P]) -> u32 {
    let mut c = s[black_box(1)];
    let cell = Cell::from_mut(&mut c.x);
    cell.set(99);
    sum(s)
}

struct Sink<'a> {
    sink: &'a Cell<u32>,
    p: P,
}

impl Drop for Sink<'_> {
    fn drop(&mut self) {
        self.p.x += 1000;
        self.sink.set(self.p.x);
    }
}

#[inline(never)]
fn drop_glue(s: &[P]) -> u32 {
    let sink = Cell::new(0);
    {
        let _d = Sink {
            sink: &sink,
            p: s[black_box(1)],
        };
    }
    sum(s) * 100000 + sink.get()
}

#[inline(never)]
fn read_back(n: u32, m: u32) -> u32 {
    let q = C { a: Cell::new(n), b: m };
    q.a.set(99);
    n * 1000 + m + q.a.get() * 1_000_000
}

#[inline(never)]
fn only_read(s: &[P]) -> u32 {
    let c = s[black_box(1)];
    let r = &c;
    let x = &c.x;
    r.y * 1000 + *x * 10 + s[1].x
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("parameters");
    print_u32(parameters(black_box(5), black_box(7)));

    print_str("slice element");
    let a = black_box([20u32, 21, 22]);
    print_u32(element(&a));
    print_u32(a[1]);

    print_str("static element");
    print_u32(static_element());
    print_u32(NUMS[1]);

    print_str("local");
    print_u32(local(black_box(5)));

    print_str("tuple field");
    let mut h = black_box((40u32, 41u32));
    print_u32(tuple_field(&mut h));
    print_u32(h.0);

    print_str("replace");
    print_u32(replace(black_box(5), black_box(7)));

    print_str("loop");
    let b = black_box([50u32, 51, 52]);
    print_u32(in_a_loop(&b));

    print_str("whole struct");
    let ps = black_box(PS);
    print_u32(whole_struct(&ps));
    print_u32(whole_struct(&PS));

    print_str("refcell");
    print_u32(refcell(black_box(5), black_box(7)));

    print_str("atomic");
    print_u32(atomic(black_box(5), black_box(7)));

    print_str("unsafecell get");
    print_u32(unsafecell_get(black_box(5), black_box(7)));

    print_str("unsafecell raw_get");
    let c = black_box([30u32, 31, 32]);
    print_u32(unsafecell_raw_get(&c));
    print_u32(c[1]);

    print_str("as_ptr");
    print_u32(as_ptr(black_box(5), black_box(7)));

    print_str("drop");
    print_u32(drop_glue(&ps));
    print_u32(drop_glue(&PS));

    print_str("read back");
    print_u32(read_back(black_box(5), black_box(7)));

    print_str("only read");
    print_u32(only_read(&ps));

    // Last before `sources`: a write that leaks into the source shows in every later sum.
    print_str("from_mut");
    print_u32(from_mut(&ps));
    print_u32(from_mut(&PS));

    print_str("sources");
    print_u32(sum(&ps));
    print_u32(sum(&PS));
}
