#!/usr/bin/env python3
"""gen_pat.py <outdir>: family C, niche widths and wrapping niches through pattern types.

One program per base integer type. A pattern type `B is lo..=hi` whose valid range ends `d` below
the type's maximum leaves niche values hi+1..=MAX and then 0..; an enum with exactly d+1 niche
variants makes rustc reserve MAX-d+1..=MAX and 0 (it crosses zero), so decoding the last variant
needs the subtraction to wrap at exactly the scalar's width. Values are made by transmute (the only
way to make a pattern-typed value); every value is in its normal representation.
"""
import os, sys

HEAD = r'''#![no_std]
#![no_main]
#![feature(pattern_types, pattern_type_macro)]
#![allow(unused, unreachable_patterns, dead_code, internal_features)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::convert::Infallible;
use core::hint::black_box;
use core::marker::PhantomData;
use core::mem::{transmute, ManuallyDrop};
use core::pattern_type;

#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] #[repr(transparent)] struct Tr<T>(T);
#[derive(Clone, Copy)] struct Named<T> { z: (), v: T, p: PhantomData<u64> }
#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }

macro_rules! suite {
    ($id:expr, $f:ident, $T:ty, [$($k:expr),* $(,)?], |$e:ident| $body:expr) => {
        #[inline(never)]
        fn $f() {
            print_i32(-(1000 + $id));
            #[inline(never)]
            fn cls($e: $T) -> u32 { $body }
            #[inline(never)]
            fn cls_o(o: Option<$T>) -> u32 { match o { Some($e) => $body, None => 55555 } }
            static TABLE: &[$T] = &[$($k),*];
            $(
                {
                    const K: $T = $k;
                    static S: $T = $k;
                    { let $e = K; print_u32($body); }
                    print_u32(cls(K));
                    print_u32(cls(black_box(K)));
                    print_u32(cls(S));
                    const O: Option<$T> = Some($k);
                    print_u32(match O { Some($e) => $body, None => 55555 });
                    print_u32(cls_o(black_box(O)));
                }
            )*
            { const N: Option<$T> = None; print_u32(cls_o(N)); print_u32(cls_o(black_box(N))); }
            let mut i = 0;
            while i < TABLE.len() {
                print_u32(cls(TABLE[black_box(i)]));
                i += 1;
            }
        }
    };
}
'''

# base type, bits, signed, printer of the base value `b`
BASES = [
    ("u8", 8, False, "b as u32"),
    ("i8", 8, True, "b as i32 as u32"),
    ("u16", 16, False, "b as u32"),
    ("i16", 16, True, "b as i32 as u32"),
    ("u32", 32, False, "b"),
    ("i32", 32, True, "b as u32"),
    ("u64", 64, False, "(b % 1_000_003) as u32"),
    ("i64", 64, True, "(b as u64 % 1_000_003) as u32"),
    ("usize", 32, False, "b as u32"),
    ("isize", 32, True, "b as u32"),
    ("u128", 128, False, "(b % 1_000_003) as u32"),
    ("i128", 128, True, "(b as u128 % 1_000_003) as u32"),
]

WRAPS = [
    ("{T}", "{v}", "{p}"),
    ("Wrap<{T}>", "Wrap({v})", "{p}.0"),
    ("Tr<{T}>", "Tr({v})", "{p}.0"),
    ("Named<{T}>", "Named {{ z: (), v: {v}, p: PhantomData }}", "{p}.v"),
    ("((), {T})", "((), {v})", "{p}.1"),
    ("SL<{T}>", "SL::Live({v})", "(match {p} {{ SL::Live(x) => x, SL::Never(..) => loop {{}} }})"),
    ("ManuallyDrop<{T}>", "ManuallyDrop::new({v})", "ManuallyDrop::into_inner({p})"),
    ("Wrap<SL<Tr<{T}>>>", "Wrap(SL::Live(Tr({v})))", "(match {p}.0 {{ SL::Live(x) => x.0, SL::Never(..) => loop {{}} }})"),
]


def lit(base, bits, signed, u):
    """the base-typed literal whose unsigned bit pattern is u"""
    if signed and u >= 1 << (bits - 1):
        return f"({u - (1 << bits)}{base})" if base not in ("isize",) else f"({u - (1 << bits)}isize)"
    return f"{u}{base}"


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for base, bits, signed, printer in BASES:
        body = HEAD
        calls = []
        idx = 0
        mx = (1 << bits) - 1
        # (lo, hi, number of unit variants) as unsigned bit patterns
        shapes = []
        for d in (1, 2, 3):
            lo, hi = mx - 60, mx - d
            for k in (d, d + 1, d + 2):
                shapes.append((lo, hi, k))
        shapes.append((1, mx, 1))            # NonZero-like: niche is 0
        shapes.append((5, mx - 1, 2))        # crosses zero with a small lo: rustc moves the start instead?
        shapes.append((0, mx - 1, 1))        # niche is MAX
        shapes.append((2, 9, 3))             # niches below the range
        for (lo, hi, k) in shapes:
            lo_l = lit(base, bits, signed, lo).strip("()")
            hi_l = lit(base, bits, signed, hi).strip("()")
            # a signed range that wraps in the unsigned view cannot be written lo..=hi
            if signed and (lo >= 1 << (bits - 1)) != (hi >= 1 << (bits - 1)):
                continue
            if signed and lo < 1 << (bits - 1) and hi >= 1 << (bits - 1):
                continue
            for wi, (wty, wval, wacc) in enumerate(WRAPS):
                if (idx + wi) % 2 and wi not in (0, 5):
                    idx += 1
                    continue
                pname = f"P{idx}"
                ename = f"E{idx}"
                pat = f"type {pname} = pattern_type!({base} is {lo_l}..={hi_l});"
                live_ty = wty.format(T=pname)
                vals = sorted({lo, hi, (lo + hi) // 2})
                consts, arms = [], []
                for v in vals:
                    mk = f"unsafe {{ transmute::<{base}, {pname}>({lit(base, bits, signed, v)}) }}"
                    consts.append(f"{ename}::Live({wval.format(v=mk)})")
                inner = wacc.format(p="w")
                arms.append(f"{ename}::Live(w) => {{ let p: {pname} = {inner}; let b: {base} = unsafe {{ transmute(p) }}; {printer} }}")
                units = []
                for i in range(k):
                    units.append(f"U{i}")
                    consts.append(f"{ename}::U{i}")
                    arms.append(f"{ename}::U{i} => {1001 + i}")
                body += pat + "\n"
                body += f"#[derive(Clone, Copy)] enum {ename} {{ Live({live_ty}), {', '.join(units)} }}\n"
                body += (f"suite!({idx}, s{idx}, {ename}, [{', '.join(consts)}], "
                         f"|e| match e {{ {', '.join(arms)} }});\n")
                calls.append(f"s{idx}();")
                idx += 1
        body += "\n#[unsafe(no_mangle)]\npub fn rust_entry() {\n    " + " ".join(calls) + "\n}\n"
        open(os.path.join(out, f"C_{base}.rs"), "w").write(body)


main()
