//! A by-value copy written through a borrow of it — handed to a callee, held by a closure,
//! reborrowed, re-viewed as a slice, or turned into a raw pointer — keeps the write to itself.
//!
//! Every such write reaches the copy through a view of the copy's own storage, and a view is a use
//! that needs the copy declared: spelled inline, the copy's clone `{ x: s.x, y: s.y }` folded under
//! the store into the source's field. Struct update syntax (`P { x: 99, ..s[1] }`) is the same
//! shape, since MIR copies the whole source first and then writes the named field. A transparent
//! `Drop` wrapper (`struct G(u32)`) and a niche-encoded `Option<&u32>` are the copies with no
//! JavaScript object of their own. `totals` reprints every source at the end.
//!
//! The moved-copy, `ptr::write`, `mem::swap`, closure, struct-update, transparent-aggregate and
//! reborrow cases are wrong on the base linker under `js-profile=release-balanced`, with and
//! without `JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on`; the others are neighbours that were
//! already right. The expectations come from this file run natively at `-O0` and `-O3`.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::hint::black_box;
use core::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy)]
struct P {
    x: u32,
    y: u32,
}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct WP(P);

#[repr(transparent)]
#[derive(Clone, Copy)]
struct W(u32);

struct C {
    a: Cell<u32>,
    b: u32,
}

static SINK: AtomicU32 = AtomicU32::new(0);

#[repr(transparent)]
struct G(u32);

impl Drop for G {
    fn drop(&mut self) {
        self.0 += 90;
        SINK.store(self.0, Ordering::Relaxed);
    }
}

struct HoldsG {
    _g: G,
    k: u32,
}

static PS: [P; 3] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }];
static AA: [[u32; 3]; 2] = [[1, 2, 3], [4, 5, 6]];
static WPS: [WP; 2] = [WP(P { x: 7, y: 8 }), WP(P { x: 9, y: 10 })];
static WS: [W; 2] = [W(11), W(12)];
static TEN: u32 = 10;
static OTHER: u32 = 777;

fn sum(s: &[P]) -> u32 {
    let mut t = 0;
    for p in s {
        t = t * 7 + p.x * 3 + p.y;
    }
    t
}

#[inline(never)]
fn set_x(p: &mut P) {
    p.x = 99;
}

#[inline(never)]
fn set_cell(c: &Cell<u32>) {
    c.set(99);
}

#[inline(never)]
fn moved_copy(s: &[P]) -> u32 {
    let c = s[black_box(1)];
    let mut d = c;
    let r = &mut d.x;
    *r = 99;
    s[1].x * 100 + c.x
}

#[inline(never)]
fn to_a_callee(s: &[P]) -> u32 {
    let mut c = s[black_box(1)];
    set_x(&mut c);
    s[1].x
}

#[inline(never)]
fn shared_to_a_callee(s: &[u32]) -> u32 {
    let q = C {
        a: Cell::new(s[black_box(1)]),
        b: 7,
    };
    set_cell(&q.a);
    s[1] + q.b
}

#[inline(never)]
fn raw_pointer(s: &[P]) -> u32 {
    let mut c = s[black_box(1)];
    let p = &mut c as *mut P;
    unsafe {
        (*p).x = 99;
    }
    s[1].x
}

#[inline(never)]
fn ptr_write(s: &[P]) -> u32 {
    let mut c = s[black_box(1)];
    unsafe {
        core::ptr::write(&mut c.x, 99);
    }
    s[1].x
}

#[inline(never)]
fn swap(s: &[P]) -> u32 {
    let mut c = s[black_box(1)];
    let mut d = P { x: 50, y: 51 };
    core::mem::swap(&mut c, &mut d);
    s[1].x * 1000 + d.x
}

#[inline(never)]
fn copy_from_slice(a: &[[u32; 3]]) -> u32 {
    let mut c = a[black_box(1)];
    c.copy_from_slice(&[9, 9, 9]);
    a[1][0]
}

#[inline(never)]
fn iter_mut(a: &[[u32; 3]]) -> u32 {
    let mut c = a[black_box(1)];
    for v in c.iter_mut() {
        *v = 99;
    }
    a[1][0] + a[1][2]
}

#[inline(never)]
fn fill(a: &[[u32; 3]]) -> u32 {
    let mut c = a[black_box(1)];
    c.fill(7);
    a[1][1]
}

#[inline(never)]
fn subslice(a: &[[u32; 3]]) -> u32 {
    let mut c = a[black_box(1)];
    let tail = &mut c[1..];
    tail[0] = 99;
    a[1][1]
}

#[inline(never)]
fn closure(s: &[P]) -> u32 {
    let mut c = s[black_box(1)];
    let mut f = || c.x = 99;
    f();
    s[1].x
}

#[inline(never)]
fn moving_closure(s: &[P]) -> u32 {
    let c = s[black_box(1)];
    let f = move || {
        let mut d = c;
        d.x = 99;
        d.x
    };
    f() + s[1].x
}

#[inline(never)]
fn struct_update(s: &[P]) -> u32 {
    let c = P {
        x: 99,
        ..s[black_box(1)]
    };
    s[1].x * 1000 + c.y
}

#[inline(never)]
fn reborrow(s: &[P]) -> u32 {
    let mut c = s[black_box(1)];
    let r1 = &mut c;
    let r2 = &mut *r1;
    r2.y = 99;
    s[1].y
}

#[inline(never)]
fn transparent_aggregate(s: &[WP]) -> u32 {
    let mut c = s[black_box(1)];
    c.0.x = 99;
    s[1].0.x
}

#[inline(never)]
fn transparent_scalar(s: &[W]) -> u32 {
    let mut c = s[black_box(1)];
    let r = &mut c.0;
    *r = 99;
    s[1].0 * 1000 + c.0
}

#[inline(never)]
fn transparent_drop(n: u32) -> u32 {
    {
        let _g = G(n);
    }
    n * 1000 + SINK.load(Ordering::Relaxed)
}

#[inline(never)]
fn transparent_drop_held(n: u32, k: u32) -> u32 {
    {
        let h = HoldsG { _g: G(n), k };
        let _ = h.k;
    }
    n * 1000 + SINK.load(Ordering::Relaxed) + k
}

#[inline(never)]
fn niche_option(s: &[Option<&'static u32>]) -> u32 {
    let mut o = s[black_box(1)];
    if let Some(r) = &mut o {
        *r = &OTHER;
    }
    *s[1].unwrap() * 10000 + *o.unwrap()
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let ps = black_box(PS);
    let aa = black_box(AA);
    let ws = black_box(WS);
    let wps = black_box(WPS);
    let options: [Option<&'static u32>; 2] = black_box([None, Some(black_box(&TEN))]);

    print_str("moved copy");
    print_u32(moved_copy(&ps));
    print_u32(moved_copy(&PS));
    print_str("to a callee");
    print_u32(to_a_callee(&ps));
    print_u32(to_a_callee(&PS));
    print_str("shared to a callee");
    let w = black_box([20u32, 21, 22]);
    print_u32(shared_to_a_callee(&w));
    print_u32(w[1]);
    print_str("raw pointer");
    print_u32(raw_pointer(&ps));
    print_u32(raw_pointer(&PS));
    print_str("ptr::write");
    print_u32(ptr_write(&ps));
    print_u32(ptr_write(&PS));
    print_str("swap");
    print_u32(swap(&ps));
    print_u32(swap(&PS));
    print_str("copy_from_slice");
    print_u32(copy_from_slice(&aa));
    print_u32(copy_from_slice(&AA));
    print_str("iter_mut");
    print_u32(iter_mut(&aa));
    print_u32(iter_mut(&AA));
    print_str("fill");
    print_u32(fill(&aa));
    print_u32(fill(&AA));
    print_str("subslice");
    print_u32(subslice(&aa));
    print_u32(subslice(&AA));
    print_str("closure");
    print_u32(closure(&ps));
    print_u32(closure(&PS));
    print_str("moving closure");
    print_u32(moving_closure(&ps));
    print_str("struct update");
    print_u32(struct_update(&ps));
    print_u32(struct_update(&PS));
    print_str("reborrow");
    print_u32(reborrow(&ps));
    print_u32(reborrow(&PS));
    print_str("transparent aggregate");
    print_u32(transparent_aggregate(&wps));
    print_u32(transparent_aggregate(&WPS));
    print_str("transparent scalar");
    print_u32(transparent_scalar(&ws));
    print_u32(transparent_scalar(&WS));
    print_str("transparent drop");
    print_u32(transparent_drop(black_box(5)));
    print_u32(transparent_drop_held(black_box(6), black_box(2)));
    print_str("niche option");
    print_u32(niche_option(&options));
    print_u32(*options[1].unwrap());

    print_str("totals");
    print_u32(sum(&ps));
    print_u32(sum(&PS));
    print_u32(AA[1][0] + AA[1][1] + AA[1][2]);
    print_u32(WPS[1].0.x + WS[1].0);
}
