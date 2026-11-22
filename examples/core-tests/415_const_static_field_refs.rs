//! Constant pointers into `static` struct, tuple, tuple-struct, and array fields/elements
//! across both JIR and v1 (A1 / MIRES-019 re-audit regression).

#![cfg_attr(not(native), no_std)]
#![cfg_attr(not(native), no_main)]

#[cfg(not(native))]
#[path = "prelude.rs"]
mod prelude;

use core::cell::Cell;
use core::hint::black_box;
use core::ptr;

#[repr(C)]
struct Pair {
    first_word: u32,
    second_word: u32,
}

static P: Pair = Pair {
    first_word: 0x1234abcd,
    second_word: 9,
};

const FIRST_OPT: Option<&u32> = Some(&P.first_word);
const SECOND_OPT: Option<&u32> = Some(&P.second_word);
const FIRST_REF: &u32 = &P.first_word;
const SECOND_REF: &u32 = &P.second_word;
const FIRST_PTR: *const u32 = &P.first_word;
const SECOND_PTR: *const u32 = &P.second_word;

static ARR: [u32; 4] = [10, 20, 30, 40];
const ARR_0: &u32 = &ARR[0];
const ARR_2: &u32 = &ARR[2];
const ARR_0_OPT: Option<&u32> = Some(&ARR[0]);
const ARR_3_OPT: Option<&u32> = Some(&ARR[3]);
const ARR_0_PTR: *const u32 = &ARR[0];
const ARR_SLICE: &[u32] = &ARR;
const ARR_SUBSLICE: &[u32] = &[ARR[1], ARR[2]];
const ARR_STATIC_SUBSLICE: Option<&[u32]> = Some(match ARR.split_at(1).1.split_at(2) {
    (mid, _) => mid,
});

static TUP: (u32, i64, bool) = (111, -222, true);
const TUP_0: &u32 = &TUP.0;
const TUP_1: &i64 = &TUP.1;
const TUP_2: &bool = &TUP.2;

struct TupStruct(u32, u32);
static TS: TupStruct = TupStruct(77, 88);
const TS_0: &u32 = &TS.0;
const TS_1: &u32 = &TS.1;

struct Reordered {
    a: u8,
    b: u32,
    c: u16,
}
static REO: Reordered = Reordered {
    a: 3,
    b: 0x102030,
    c: 0x0506,
};
const REO_A: &u8 = &REO.a;
const REO_B: &u32 = &REO.b;
const REO_C: &u16 = &REO.c;

#[allow(non_snake_case)]
struct SpecialKeys {
    class: u32,
    TAG: u32,
}
static SPECIAL: SpecialKeys = SpecialKeys { class: 31, TAG: 47 };
const SP_CLASS: &u32 = &SPECIAL.class;
const SP_TAG: &u32 = &SPECIAL.TAG;

#[repr(transparent)]
struct SyncCell<T>(Cell<T>);
unsafe impl<T> Sync for SyncCell<T> {}

struct Counters {
    a: SyncCell<u32>,
    b: SyncCell<u32>,
}
static COUNTERS: Counters = Counters {
    a: SyncCell(Cell::new(5)),
    b: SyncCell(Cell::new(15)),
};
const C_A: Option<&Cell<u32>> = Some(&COUNTERS.a.0);
const C_B: &SyncCell<u32> = &COUNTERS.b;

static LINKED_REF: &u32 = &P.second_word;
static LINKED_OPT: Option<&u32> = Some(&P.first_word);

#[inline(never)]
fn bump(x: Option<&u32>) -> u32 {
    x.map_or(100, |v| v.wrapping_add(1))
}

#[inline(always)]
fn inlined_first() -> Option<&'static u32> {
    FIRST_OPT
}

#[inline(always)]
fn inlined_second() -> &'static u32 {
    SECOND_REF
}

fn out_u32(v: u32) {
    #[cfg(native)]
    println!("{v}");
    #[cfg(not(native))]
    prelude::print_u32(v);
}

fn out_i64(v: i64) {
    #[cfg(native)]
    println!("{v}");
    #[cfg(not(native))]
    prelude::print_i64(v);
}

fn out_bool(v: bool) {
    #[cfg(native)]
    println!("{v}");
    #[cfg(not(native))]
    prelude::print_bool(v);
}

#[inline(never)]
fn check_static_slices() {
    out_u32(black_box(ARR_SLICE).len() as u32);
    out_u32(black_box(ARR_SLICE)[3]);
    out_u32(black_box(ARR_SUBSLICE)[0] + black_box(ARR_SUBSLICE)[1]);
    let mid = black_box(ARR_STATIC_SUBSLICE).unwrap();
    out_u32(mid.len() as u32);
    out_u32(mid[0]);
    out_u32(mid[1]);
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Exact field.rs reproducer and sibling field
    out_u32(bump(black_box(FIRST_OPT)));
    out_u32(bump(black_box(SECOND_OPT)));
    out_u32(*black_box(FIRST_REF));
    out_u32(*black_box(SECOND_REF));
    out_u32(bump(black_box(inlined_first())));
    out_u32(*black_box(inlined_second()));

    // Static array elements
    out_u32(*black_box(ARR_0));
    out_u32(*black_box(ARR_2));
    out_u32(bump(black_box(ARR_0_OPT)));
    out_u32(bump(black_box(ARR_3_OPT)));
    check_static_slices();

    // Static tuple elements
    out_u32(*black_box(TUP_0));
    out_i64(*black_box(TUP_1));
    out_bool(*black_box(TUP_2));

    // Static tuple struct fields (_0, _1)
    out_u32(*black_box(TS_0));
    out_u32(*black_box(TS_1));

    // Reordered Rust struct fields
    out_u32(u32::from(*black_box(REO_A)));
    out_u32(*black_box(REO_B));
    out_u32(u32::from(*black_box(REO_C)));

    // Reserved-word / TAG escaped struct fields
    out_u32(*black_box(SP_CLASS));
    out_u32(*black_box(SP_TAG));

    // Interior mutability aliasing live static storage
    COUNTERS.a.0.set(50);
    out_u32(black_box(C_A).unwrap().get());
    black_box(C_A).unwrap().set(55);
    out_u32(COUNTERS.a.0.get());
    black_box(C_B).0.set(155);
    out_u32(COUNTERS.b.0.get());

    // Static-to-static field references (dependency ordering)
    out_u32(*black_box(LINKED_REF));
    out_u32(bump(black_box(LINKED_OPT)));

    // Raw pointers + ptr::eq + array pointer arithmetic
    let p1 = black_box(FIRST_PTR);
    let p2 = black_box(SECOND_PTR);
    let a0 = black_box(ARR_0_PTR);
    unsafe {
        out_u32(*p1);
        out_u32(*p2);
        out_u32(*a0.add(2));
    }
    out_bool(ptr::eq(p1, &P.first_word));
    out_bool(ptr::eq(p1, &raw const P.first_word));
    out_bool(ptr::eq(p2, &P.second_word));
    out_bool(ptr::eq(p1, p2));
    out_bool(ptr::eq(a0, &ARR[0]));
    out_bool(ptr::eq(unsafe { a0.add(2) }, &ARR[2]));
}

#[cfg(native)]
fn main() {
    rust_entry();
}
