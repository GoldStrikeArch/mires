#!/usr/bin/env python3
"""gen_one.py <outdir>: one program per element kind of `static ONE: &[T] = slice::from_ref(&S)`.
Each prints the length, pointer identity with the static, identity of a second constant slice and of
the run-time slice, and one value read through the slice."""
import sys
from pathlib import Path

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)

# name, extra items, type, initializer, expression over `v: &T` printing something
CASES = [
    ("unit", "", "()", "()", "print_u32(1)"),
    ("unit_struct", "struct U;", "U", "U", "print_u32(1)"),
    ("phantom", "", "core::marker::PhantomData<u32>", "core::marker::PhantomData", "print_u32(1)"),
    ("empty_array", "", "[u32; 0]", "[]", "print_usize(v.len())"),
    ("zst_tuple", "", "((), ())", "((), ())", "print_u32(1)"),
    ("zst_struct_fields", "struct Z { a: (), b: [u8; 0] }", "Z", "Z { a: (), b: [] }", "print_usize(v.b.len())"),
    ("newtype", "struct W(u32);", "W", "W(5)", "print_u32(v.0)"),
    ("transparent", "#[repr(transparent)] struct T(u32);", "T", "T(6)", "print_u32(v.0)"),
    ("transparent_agg", "struct P { x: u32 } #[repr(transparent)] struct T(P);", "T", "T(P { x: 7 })", "print_u32(v.0.x)"),
    ("transparent_zst", "#[repr(transparent)] struct T(u32, ());", "T", "T(8, ())", "print_u32(v.0)"),
    ("nonzero", "", "core::num::NonZeroU32", "core::num::NonZeroU32::new(9).unwrap()", "print_u32(v.get())"),
    ("wrapping", "", "core::num::Wrapping<u32>", "core::num::Wrapping(10)", "print_u32(v.0)"),
    ("fieldless_enum", "enum C { R, G, B }", "C", "C::G", "print_u32(match v { C::R => 0, C::G => 1, C::B => 2 })"),
    ("fieldless_enum_u8", "#[repr(u8)] enum C { R = 3, G = 7 }", "C", "C::G", "print_u32(match v { C::R => 0, C::G => 1 })"),
    ("one_variant_enum", "enum O { Only }", "O", "O::Only", "print_u32(1)"),
    ("ordering", "", "core::cmp::Ordering", "core::cmp::Ordering::Greater", "print_bool(v.is_gt())"),
    ("option_u32", "", "Option<u32>", "Some(11)", "print_u32(v.unwrap_or(0))"),
    ("option_nonzero", "", "Option<core::num::NonZeroU32>", "core::num::NonZeroU32::new(12)", "print_u32(v.map_or(0, |n| n.get()))"),
    ("option_bool", "", "Option<bool>", "Some(true)", "print_bool(v.unwrap_or(false))"),
    ("option_none", "", "Option<u64>", "None", "print_bool(v.is_none())"),
    ("option_ref", "static T: u32 = 13;", "Option<&'static u32>", "Some(&T)", "print_u32(v.map_or(0, |r| *r))"),
    ("option_unit", "", "Option<()>", "Some(())", "print_bool(v.is_some())"),
    ("result", "", "Result<u32, u8>", "Err(14)", "print_u32(match v { Ok(x) => *x, Err(e) => 1000 + *e as u32 })"),
    ("u128", "", "u128", "1 << 100", "print_u64((*v >> 64) as u64)"),
    ("i128", "", "i128", "-5", "print_i64(*v as i64)"),
    ("i64", "", "i64", "-(1 << 40)", "print_i64(*v)"),
    ("f32", "", "f32", "1.5", "print_f64(*v as f64)"),
    ("u8", "", "u8", "200", "print_u32(*v as u32)"),
    ("str_ref", "", "&'static str", "\"text\"", "print_str(v)"),
    ("slice_ref", "", "&'static [u32]", "&[1, 2, 3]", "print_usize(v.len())"),
    ("array_ref", "", "&'static [u32; 2]", "&[4, 5]", "print_u32(v[1])"),
    ("fn_ptr", "fn f(x: u32) -> u32 { x + 1 }", "fn(u32) -> u32", "f", "print_u32(v(15))"),
    ("closure_fn_ptr", "", "fn(u32) -> u32", "|x| x + 2", "print_u32(v(15))"),
    ("tuple1", "", "(u32,)", "(16,)", "print_u32(v.0)"),
    ("nested_array", "", "[[u8; 2]; 2]", "[[1, 2], [3, 4]]", "print_u32(v[1][0] as u32)"),
    ("array_of_unit", "", "[(); 3]", "[(); 3]", "print_usize(v.len())"),
    ("bool", "", "bool", "true", "print_bool(*v)"),
    ("char", "", "char", "'q'", "print_u32(*v as u32)"),
    ("atomic", "", "core::sync::atomic::AtomicU32", "core::sync::atomic::AtomicU32::new(17)", "print_u32(v.load(core::sync::atomic::Ordering::Relaxed))"),
    ("atomic_bool", "", "core::sync::atomic::AtomicBool", "core::sync::atomic::AtomicBool::new(true)", "print_bool(v.load(core::sync::atomic::Ordering::Relaxed))"),
    ("atomic_u64", "", "core::sync::atomic::AtomicU64", "core::sync::atomic::AtomicU64::new(1 << 40)", "print_u64(v.load(core::sync::atomic::Ordering::Relaxed))"),
    ("manually_drop", "", "core::mem::ManuallyDrop<u32>", "core::mem::ManuallyDrop::new(18)", "print_u32(**v)"),
    ("maybe_uninit", "", "core::mem::MaybeUninit<u32>", "core::mem::MaybeUninit::new(19)", "print_u32(unsafe { v.assume_init() })"),
    ("union", "#[derive(Clone, Copy)] union Un { a: u32, b: f32 }", "Un", "Un { a: 20 }", "print_u32(unsafe { v.a })"),
]

TEMPLATE = """//! One-element slice over a `static` of element kind `{name}`.
#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::ptr;
use core::slice::from_ref;

{extra}
type T0 = {ty};
static S: T0 = {init};
static ONE: &[T0] = from_ref(&S);
static TWO: &[T0] = from_ref(&S);
static OPT: Option<&[T0]> = Some(from_ref(&S));

#[inline(never)]
fn show(v: &T0) {{ {show}; }}

#[unsafe(no_mangle)]
pub fn rust_entry() {{
    print_usize(ONE.len());
    print_usize(black_box(ONE).iter().count());
    show(&ONE[0]);
    show(&S);
    for v in TWO {{ show(v); }}
    print_usize(OPT.map_or(9, |s| s.len()));
    if core::mem::size_of::<T0>() != 0 {{
        print_bool(ptr::eq(ONE.as_ptr(), &S));
        print_bool(ptr::eq(&ONE[0], &S));
        print_bool(ptr::eq(ONE.as_ptr(), TWO.as_ptr()));
        print_bool(ptr::eq(ONE.as_ptr(), from_ref(black_box(&S)).as_ptr()));
        print_bool(OPT.map_or(false, |s| ptr::eq(s.as_ptr(), &S)));
    }}
}}
"""

for name, extra, ty, init, show in CASES:
    (out / f"one_{name}.rs").write_text(
        TEMPLATE.format(name=name, extra=extra, ty=ty, init=init, show=show)
    )
print(len(CASES))
