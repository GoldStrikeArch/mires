//! Constant and `static` `&dyn Trait` / `*const dyn Trait` / `NonNull<dyn Trait>` fat pointers
//! decoded from allocations across both JIR and baseline lanes (MIRES-024 follow-up).
//!
//! Covers promoted `&fn_item`, non-capturing closures, `FnPtr`, structs, direct primitives, ZSTs,
//! `&str`, principal-less `dyn Sync`, const-time & runtime trait upcasting, slices and structs
//! holding `&dyn`, `Option<&dyn>` and `Option<NonNull<dyn>>`, interior-mutable statics (both
//! indirect structs and direct boxed `repr(transparent)` cells), null raw `*const dyn`, and JIR
//! cross-function inlining of `ConstTree::Dyn`.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::cell::Cell;
use core::hint::black_box;
use core::mem::{align_of_val, size_of_val};
use core::ptr::NonNull;

#[inline(never)]
fn twice(x: u32) -> u32 {
    x * 2
}

#[inline(never)]
fn plus1(x: u32) -> u32 {
    x + 1
}

trait Super: Sync {
    fn super_val(&self) -> u32;
}

trait Sub: Super {
    fn sub_val(&self) -> u32;
}

trait Calc: Sync {
    fn eval(&self, x: u32) -> u32;
}

struct Adder(u32);
impl Calc for Adder {
    fn eval(&self, x: u32) -> u32 {
        self.0 + x
    }
}
impl Super for Adder {
    fn super_val(&self) -> u32 {
        self.0 + 100
    }
}
impl Sub for Adder {
    fn sub_val(&self) -> u32 {
        self.0 + 200
    }
}

impl Calc for u32 {
    fn eval(&self, x: u32) -> u32 {
        *self * x
    }
}

impl Calc for &str {
    fn eval(&self, x: u32) -> u32 {
        self.len() as u32 + x
    }
}

struct ZstCalc;
impl Calc for ZstCalc {
    fn eval(&self, x: u32) -> u32 {
        x + 1000
    }
}

struct SyncCellStruct {
    v: Cell<u32>,
}
unsafe impl Sync for SyncCellStruct {}
impl Calc for SyncCellStruct {
    fn eval(&self, x: u32) -> u32 {
        let old = self.v.get();
        self.v.set(old + x);
        self.v.get()
    }
}

#[repr(transparent)]
struct SyncCellU32(Cell<u32>);
unsafe impl Sync for SyncCellU32 {}
impl Calc for SyncCellU32 {
    fn eval(&self, x: u32) -> u32 {
        let old = self.0.get();
        self.0.set(old + x);
        self.0.get()
    }
}

struct DynBox {
    f: &'static dyn Fn(u32) -> u32,
    arg: u32,
}

const REF_FN: &dyn Fn(u32) -> u32 = &twice;
static STAT_DYN_FN: &(dyn Fn(u32) -> u32 + Sync) = &twice;
const REF_CL: &dyn Fn(u32) -> u32 = &|x| x + 10;
const C_DYN_FNPTR: &dyn Fn(u32) -> u32 = &(twice as fn(u32) -> u32);
const C_DYN_STRUCT: &dyn Calc = &Adder(5);
const C_DYN_PRIM: &dyn Calc = &4u32;
const C_DYN_ZST: &dyn Calc = &ZstCalc;
static S_DYN_STRUCT: &dyn Calc = &Adder(7);
const C_DYN_ARR: &[&dyn Fn(u32) -> u32] = &[&twice, &plus1, &|x| x * 3];
const C_OPT_DYN_SOME: Option<&dyn Fn(u32) -> u32> = Some(&twice);
const C_OPT_DYN_NONE: Option<&dyn Fn(u32) -> u32> = None;
const C_DYN_BOX: DynBox = DynBox { f: &twice, arg: 9 };
const C_DYN_SYNC_U32: &dyn Sync = &42u32;
const C_DYN_SYNC_FN: &dyn Sync = &twice;
const C_SUB: &dyn Sub = &Adder(7);
const C_SUP_FROM_SUB: &dyn Super = C_SUB;
const C_RAW_DYN_FN: *const dyn Fn(u32) -> u32 = &twice;
const C_RAW_DYN_STRUCT: *const dyn Calc = &Adder(11);
const C_RAW_DYN_PRIM: *const dyn Calc = &3u32;
const C_NULL_DYN: *const dyn Calc = core::ptr::null::<Adder>() as *const dyn Calc;
const C_NN_DYN: NonNull<dyn Calc> = NonNull::from_ref(&Adder(12) as &dyn Calc);
const C_OPT_NN_SOME: Option<NonNull<dyn Calc>> = Some(C_NN_DYN);
const C_OPT_NN_NONE: Option<NonNull<dyn Calc>> = None;
static S_OPT_DYN_SOME: Option<&(dyn Fn(u32) -> u32 + Sync)> = Some(&twice);
static S_OPT_DYN_NONE: Option<&(dyn Fn(u32) -> u32 + Sync)> = None;
static S_CELL_STRUCT: SyncCellStruct = SyncCellStruct { v: Cell::new(10) };
const C_REF_CELL_STRUCT: &dyn Calc = &S_CELL_STRUCT;
static S_CELL_U32: SyncCellU32 = SyncCellU32(Cell::new(20));
const C_REF_CELL_U32: &dyn Calc = &S_CELL_U32;
static S_STR: &str = "hello";
const C_DYN_STATIC_STR: &dyn Calc = &S_STR;
const C_DYN_PROMOTED_STR: &dyn Calc = &"world!";

#[inline(never)]
fn call_dyn(f: &dyn Fn(u32) -> u32, x: u32) -> u32 {
    f(x)
}

#[inline(always)]
fn get_inlined_dyn_fn() -> &'static dyn Fn(u32) -> u32 {
    const F: &dyn Fn(u32) -> u32 = &twice;
    F
}

#[inline(always)]
fn get_inlined_dyn_static() -> &'static dyn Calc {
    C_REF_CELL_U32
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(call_dyn(REF_FN, 7));
    print_u32(call_dyn(STAT_DYN_FN, 8));
    print_u32(call_dyn(REF_CL, 9));
    print_u32(C_DYN_FNPTR(9));
    print_u32(C_DYN_STRUCT.eval(10));
    print_u32(C_DYN_PRIM.eval(10));
    print_u32(C_DYN_ZST.eval(10));
    print_u32(S_DYN_STRUCT.eval(10));
    print_u32(C_DYN_ARR[0](2) + C_DYN_ARR[1](2) + C_DYN_ARR[2](2));
    print_u32(match black_box(C_OPT_DYN_SOME) {
        Some(f) => f(4),
        None => 0,
    });
    print_u32(match black_box(C_OPT_DYN_NONE) {
        Some(f) => f(4),
        None => 99,
    });
    print_u32((C_DYN_BOX.f)(C_DYN_BOX.arg));

    print_u32(size_of_val(black_box(C_DYN_SYNC_U32)) as u32);
    print_u32(align_of_val(black_box(C_DYN_SYNC_U32)) as u32);
    print_u32(size_of_val(black_box(C_DYN_SYNC_FN)) as u32);
    print_u32(align_of_val(black_box(C_DYN_SYNC_FN)) as u32);

    print_u32(C_SUB.sub_val());
    print_u32(C_SUB.super_val());
    print_u32(C_SUP_FROM_SUB.super_val());

    print_bool(!black_box(C_RAW_DYN_FN).is_null());
    print_u32(unsafe { (&*black_box(C_RAW_DYN_FN))(6) });
    print_u32(unsafe { (&*black_box(C_RAW_DYN_STRUCT)).eval(4) });
    print_u32(unsafe { (&*black_box(C_RAW_DYN_PRIM)).eval(5) });
    print_bool(black_box(C_NULL_DYN).is_null());

    print_u32(unsafe { C_NN_DYN.as_ref().eval(3) });
    print_u32(match black_box(C_OPT_NN_SOME) {
        Some(p) => unsafe { p.as_ref().eval(2) },
        None => 0,
    });
    print_u32(match black_box(C_OPT_NN_NONE) {
        Some(p) => unsafe { p.as_ref().eval(2) },
        None => 88,
    });
    print_u32(match black_box(S_OPT_DYN_SOME) {
        Some(f) => f(13),
        None => 0,
    });
    print_u32(match black_box(S_OPT_DYN_NONE) {
        Some(f) => f(13),
        None => 77,
    });

    print_u32(C_REF_CELL_STRUCT.eval(5));
    print_u32(S_CELL_STRUCT.v.get());
    S_CELL_STRUCT.v.set(30);
    print_u32(C_REF_CELL_STRUCT.eval(7));
    print_u32(C_REF_CELL_U32.eval(4));
    print_u32(S_CELL_U32.0.get());
    S_CELL_U32.0.set(50);
    print_u32(get_inlined_dyn_static().eval(6));
    print_u32(S_CELL_U32.0.get());

    print_u32(C_DYN_STATIC_STR.eval(1));
    print_u32(C_DYN_PROMOTED_STR.eval(1));
    print_u32(get_inlined_dyn_fn()(21));
    print_u32(size_of_val(black_box(&ZstCalc as &dyn Calc)) as u32);
    print_u32(size_of_val(black_box(unsafe { &*C_RAW_DYN_STRUCT })) as u32);
}
