//! `transmute` rebuild for ZST-padded tuples, tuple structs, nested wrappers, 1-element arrays,
//! and niche-encoded enums (`Option` / `Result`) carrying ZST-padded tuple payloads.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::mem::{size_of, transmute};
use core::num::{NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI8, NonZeroU8};

type OptPaddedI16 = Option<((), NonZeroI16, ())>;
type OptTrailU8 = Option<((), (), NonZeroU8)>;
type OptNested = Option<((), ((), NonZeroI16, ()), ())>;
type OptZstArr = Option<([(); 3], [NonZeroI8; 1], ())>;
type ResPadded = Result<((), NonZeroI16, ()), ()>;
type OptPaddedI64 = Option<((), NonZeroI64, ())>;

struct TupWrap((), NonZeroI32, ());
struct NestedWrap((), ((), NonZeroI16, ()), ());
struct Named {
    _a: (),
    v: NonZeroI16,
    _b: (),
}

const _: () = assert!(size_of::<OptPaddedI16>() == 2);
const _: () = assert!(size_of::<OptTrailU8>() == 1);
const _: () = assert!(size_of::<OptNested>() == 2);
const _: () = assert!(size_of::<OptZstArr>() == 1);
const _: () = assert!(size_of::<ResPadded>() == 2);
const _: () = assert!(size_of::<OptPaddedI64>() == 8);
const _: () = assert!(size_of::<Option<TupWrap>>() == 4);
const _: () = assert!(size_of::<Option<NestedWrap>>() == 2);
const _: () = assert!(size_of::<Option<Named>>() == 2);

#[inline(never)]
fn encode_opt_i16(x: OptPaddedI16) -> i16 {
    unsafe { transmute(x) }
}
#[inline(never)]
fn decode_opt_i16(x: i16) -> OptPaddedI16 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn encode_direct_tuple(x: ((), i16, ())) -> i16 {
    unsafe { transmute(x) }
}
#[inline(never)]
fn decode_direct_tuple(x: i16) -> ((), i16, ()) {
    unsafe { transmute(x) }
}

#[inline(never)]
fn swap_zst_side(x: ((), i16)) -> (i16, ()) {
    unsafe { transmute(x) }
}
#[inline(never)]
fn swap_zst_back(x: (i16, ())) -> ((), i16) {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_trail_u8(x: u8) -> ((), (), NonZeroU8) {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_trail_u8(x: ((), (), NonZeroU8)) -> u8 {
    unsafe { transmute(x) }
}
#[inline(never)]
fn decode_opt_trail_u8(x: u8) -> OptTrailU8 {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_opt_trail_u8(x: OptTrailU8) -> u8 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_tup_wrap(x: i32) -> Option<TupWrap> {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_tup_wrap(x: Option<TupWrap>) -> i32 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_opt_nested(x: i16) -> OptNested {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_opt_nested(x: OptNested) -> i16 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_nested_wrap(x: i16) -> Option<NestedWrap> {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_nested_wrap(x: Option<NestedWrap>) -> i16 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_opt_zst_arr(x: i8) -> OptZstArr {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_opt_zst_arr(x: OptZstArr) -> i8 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_arr_pair(x: i16) -> ((), [NonZeroI16; 1]) {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_arr_pair(x: ((), [NonZeroI16; 1])) -> i16 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_res_padded(x: i16) -> ResPadded {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_res_padded(x: ResPadded) -> i16 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_named(x: i16) -> Option<Named> {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_named(x: Option<Named>) -> i16 {
    unsafe { transmute(x) }
}

#[inline(never)]
fn decode_opt_i64(x: i64) -> OptPaddedI64 {
    unsafe { transmute(x) }
}
#[inline(never)]
fn encode_opt_i64(x: OptPaddedI64) -> i64 {
    unsafe { transmute(x) }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // 1. Exact `tuple.rs` repro shape: `Option<((), NonZeroI16, ())>`.
    for x in [
        None,
        Some(((), NonZeroI16::new(-913).unwrap(), ())),
        Some(((), NonZeroI16::new(17).unwrap(), ())),
    ] {
        let bits = encode_opt_i16(black_box(x));
        print_i32(bits as i32);
        let back = decode_opt_i16(black_box(bits));
        print_i32(back.map_or(42, |tuple| tuple.1.get() as i32));
    }

    // 2. Direct tuple transmutes (`((), i16, ())`, `((), i16)` <-> `(i16, ())`).
    for v in [-32768i16, 0, 1234, 32767] {
        let t = decode_direct_tuple(black_box(v));
        print_i32(t.1 as i32);
        print_i32(encode_direct_tuple(black_box(t)) as i32);
        let a = swap_zst_side(black_box(((), v)));
        print_i32(a.0 as i32);
        let b = swap_zst_back(black_box(a));
        print_i32(b.1 as i32);
    }

    // 3. Trailing non-ZST at index 2: `((), (), NonZeroU8)` and `Option<((), (), NonZeroU8)>`.
    let tu = decode_trail_u8(black_box(205u8));
    print_i32(tu.2.get() as i32);
    print_i32(encode_trail_u8(black_box(tu)) as i32);
    for x in [None, Some(((), (), NonZeroU8::new(77).unwrap()))] {
        let bits = encode_opt_trail_u8(black_box(x));
        print_i32(bits as i32);
        let back = decode_opt_trail_u8(black_box(bits));
        print_i32(back.map_or(-1, |t| t.2.get() as i32));
    }

    // 4. Tuple struct `TupWrap((), NonZeroI32, ())`.
    for x in [None, Some(TupWrap((), NonZeroI32::new(-123456).unwrap(), ()))] {
        let bits = encode_tup_wrap(black_box(x));
        print_i32(bits);
        let back = decode_tup_wrap(black_box(bits));
        print_i32(back.map_or(99, |w| w.1.get()));
    }

    // 5. Nested ZST-padded tuple and struct-around-tuple.
    for x in [
        None,
        Some(((), ((), NonZeroI16::new(-444).unwrap(), ()), ())),
    ] {
        let bits = encode_opt_nested(black_box(x));
        print_i32(bits as i32);
        let back = decode_opt_nested(black_box(bits));
        print_i32(back.map_or(55, |outer| (outer.1).1.get() as i32));
    }
    for x in [
        None,
        Some(NestedWrap((), ((), NonZeroI16::new(888).unwrap(), ()), ())),
    ] {
        let bits = encode_nested_wrap(black_box(x));
        print_i32(bits as i32);
        let back = decode_nested_wrap(black_box(bits));
        print_i32(back.map_or(66, |w| (w.1).1.get() as i32));
    }

    // 6. ZST array + 1-element array inside tuple.
    for x in [None, Some(([(); 3], [NonZeroI8::new(-42).unwrap()], ()))] {
        let bits = encode_opt_zst_arr(black_box(x));
        print_i32(bits as i32);
        let back = decode_opt_zst_arr(black_box(bits));
        print_i32(back.map_or(77, |t| t.1[0].get() as i32));
    }
    let ap = decode_arr_pair(black_box(-515i16));
    print_i32(ap.1[0].get() as i32);
    print_i32(encode_arr_pair(black_box(ap)) as i32);

    // 7. `Result<((), NonZeroI16, ()), ()>`.
    for x in [Err(()), Ok(((), NonZeroI16::new(-222).unwrap(), ()))] {
        let bits = encode_res_padded(black_box(x));
        print_i32(bits as i32);
        let back = decode_res_padded(black_box(bits));
        print_i32(match back {
            Ok(t) => t.1.get() as i32,
            Err(()) => 333,
        });
    }

    // 8. Named struct with leading/trailing ZST fields.
    for x in [
        None,
        Some(Named {
            _a: (),
            v: NonZeroI16::new(-777).unwrap(),
            _b: (),
        }),
    ] {
        let bits = encode_named(black_box(x));
        print_i32(bits as i32);
        let back = decode_named(black_box(bits));
        print_i32(back.map_or(88, |n| n.v.get() as i32));
    }

    // 9. 64-bit `Option<((), NonZeroI64, ())>`.
    for x in [
        None,
        Some(((), NonZeroI64::new(-9_000_000_001).unwrap(), ())),
        Some(((), NonZeroI64::new(123_456_789_012).unwrap(), ())),
    ] {
        let bits = encode_opt_i64(black_box(x));
        print_i64(bits);
        let back = decode_opt_i64(black_box(bits));
        print_i64(back.map_or(42, |t| t.1.get()));
    }
}
