//! Nested struct/tuple/array projections, raw aggregate pointers, ZST siblings, and `&dyn Trait`
//! constants pointing into `static` fields/elements across both JIR and v1.

#![cfg_attr(not(native), no_std)]
#![cfg_attr(not(native), no_main)]

#[cfg(not(native))]
#[path = "prelude.rs"]
mod prelude;

use core::cell::Cell;
use core::hint::black_box;
use core::ptr::{self, NonNull};

struct Inner {
    val: u32,
    tag: u16,
}

struct Outer {
    inner: Inner,
    arr: [Inner; 2],
    tup: (u32, Inner),
}

static OUTER: Outer = Outer {
    inner: Inner { val: 42, tag: 7 },
    arr: [Inner { val: 100, tag: 1 }, Inner { val: 200, tag: 2 }],
    tup: (999, Inner { val: 300, tag: 3 }),
};

const NESTED_VAL: &u32 = &OUTER.inner.val;
const NESTED_TAG: Option<&u16> = Some(&OUTER.inner.tag);
const ARR_1_VAL: &u32 = &OUTER.arr[1].val;
const TUP_0: &u32 = &OUTER.tup.0;
const TUP_INNER_OPT: Option<&Inner> = Some(&OUTER.tup.1);
const INNER_OPT: Option<&Inner> = Some(&OUTER.inner);
const ARR_REF: &[Inner; 2] = &OUTER.arr;

const INNER_PTR: *const Inner = &raw const OUTER.inner;
const ARR_0_PTR: *const Inner = &raw const OUTER.arr[0];
const INNER_NN: Option<NonNull<Inner>> = Some(NonNull::from_ref(&OUTER.inner));

trait Calc: Sync {
    fn calc(&self, x: u32) -> u32;
}

impl Calc for Inner {
    fn calc(&self, x: u32) -> u32 {
        self.val + u32::from(self.tag) * 10 + x
    }
}

impl Calc for u32 {
    fn calc(&self, x: u32) -> u32 {
        *self * 2 + x
    }
}

const DYN_INNER: &dyn Calc = &OUTER.inner;
const DYN_ARR1: &dyn Calc = &OUTER.arr[1];
const DYN_PRIM: &dyn Calc = &OUTER.inner.val;
const DYN_OPT: Option<&dyn Calc> = Some(&OUTER.tup.1);
const DYN_SLICE: &[&dyn Calc] = &[&OUTER.inner, &OUTER.arr[0], &OUTER.inner.val];
static DYN_STATIC: &dyn Calc = &OUTER.arr[1];

#[repr(transparent)]
struct SyncCell<T>(Cell<T>);
unsafe impl<T> Sync for SyncCell<T> {}

struct LiveInner {
    v: SyncCell<u32>,
}

impl Calc for LiveInner {
    fn calc(&self, x: u32) -> u32 {
        self.v.0.get() + x
    }
}

struct LiveOuter {
    pad: u32,
    inner: LiveInner,
}

static LIVE: LiveOuter = LiveOuter {
    pad: 1,
    inner: LiveInner {
        v: SyncCell(Cell::new(10)),
    },
};

const LIVE_DYN: &dyn Calc = &LIVE.inner;
const LIVE_PAD: &u32 = &LIVE.pad;

struct WithZst {
    z: (),
    x: u32,
}

static WZ: WithZst = WithZst { z: (), x: 616 };
const WZ_X: Option<&u32> = Some(&WZ.x);
const WZ_Z: Option<&()> = Some(&WZ.z);

#[inline(never)]
fn id_ref(r: &Inner) -> &Inner {
    r
}

#[inline(always)]
fn inlined_nested_dyn() -> &'static dyn Calc {
    DYN_INNER
}

#[inline(always)]
fn inlined_nested_val() -> &'static u32 {
    ARR_1_VAL
}

fn out_u32(v: u32) {
    #[cfg(native)]
    println!("{v}");
    #[cfg(not(native))]
    prelude::print_u32(v);
}

fn out_bool(v: bool) {
    #[cfg(native)]
    println!("{v}");
    #[cfg(not(native))]
    prelude::print_bool(v);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Deep nested projections
    out_u32(*black_box(NESTED_VAL));
    out_u32(u32::from(*black_box(NESTED_TAG).unwrap()));
    out_u32(*black_box(ARR_1_VAL));
    out_u32(*black_box(TUP_0));
    out_u32(black_box(TUP_INNER_OPT).unwrap().val);
    out_u32(black_box(INNER_OPT).unwrap().val);
    out_u32(black_box(ARR_REF)[0].val + black_box(ARR_REF)[1].val);
    out_u32(*black_box(inlined_nested_val()));

    // Raw pointers to nested aggregates + ptr::eq + pointer arithmetic
    let ip = black_box(INNER_PTR);
    let ap = black_box(ARR_0_PTR);
    unsafe {
        out_u32((*ip).val);
        out_u32((*ap).val);
        out_u32((*ap.add(1)).val);
        out_u32(black_box(INNER_NN).unwrap().as_ref().val);
    }
    out_bool(ptr::eq(ip, &raw const OUTER.inner));
    out_bool(ptr::eq(black_box(INNER_OPT).unwrap(), id_ref(&OUTER.inner)));
    out_bool(ptr::eq(ap, &raw const OUTER.arr[0]));
    out_bool(ptr::eq(unsafe { ap.add(1) }, &raw const OUTER.arr[1]));
    out_bool(ptr::eq(ap, &raw const OUTER.arr[1]));

    // &dyn Trait constants pointing into static sub-fields
    out_u32(black_box(DYN_INNER).calc(3));
    out_u32(black_box(DYN_ARR1).calc(4));
    out_u32(black_box(DYN_PRIM).calc(5));
    out_u32(black_box(DYN_OPT).unwrap().calc(6));
    out_u32(black_box(inlined_nested_dyn()).calc(7));
    out_u32(black_box(DYN_STATIC).calc(8));
    let mut sum = 0u32;
    for d in black_box(DYN_SLICE) {
        sum += d.calc(1);
    }
    out_u32(sum);

    // Interior mutability through &dyn Trait pointing at a static sub-field
    out_u32(*black_box(LIVE_PAD));
    out_u32(black_box(LIVE_DYN).calc(1));
    LIVE.inner.v.0.set(777);
    out_u32(black_box(LIVE_DYN).calc(1));

    // ZST field at offset 0 alongside non-ZST field
    out_u32(*black_box(WZ_X).unwrap());
    out_bool(black_box(WZ_Z).is_some());
}

#[cfg(native)]
fn main() {
    rust_entry();
}
