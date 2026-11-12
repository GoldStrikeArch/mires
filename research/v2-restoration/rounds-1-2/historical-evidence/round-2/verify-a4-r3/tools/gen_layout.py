#!/usr/bin/env python3
"""gen_layout.py <outdir> <seed> <n_enums> <per_file>

Family L: randomly generated enums whose layout is a niche or a single variant, read back from
constants (inline, argument, black_box, promoted, static, static table) and through Option /
Option<Option> wrappers. Truth is the native run. Every suite prints a marker `-(1000+id)` first so
a diff localises to one enum.
"""
import os, random, sys

HEAD = r'''#![no_std]
#![no_main]
#![allow(unused, unreachable_patterns, dead_code)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::cmp::Ordering;
use core::convert::Infallible;
use core::hint::black_box;
use core::marker::PhantomData;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroI16, NonZeroU64, NonZeroI8};

trait HasOut { type Out; }
impl<T> HasOut for fn() -> T { type Out = T; }
type Never = <fn() -> ! as HasOut>::Out;

#[derive(Clone, Copy)] enum Void {}
#[derive(Clone, Copy)] struct Wrap<T>(T);
#[derive(Clone, Copy)] #[repr(transparent)] struct Tr<T>(T);
#[derive(Clone, Copy)] #[repr(C)] struct CW<T>(T);
#[derive(Clone, Copy)] #[repr(packed)] struct Pk<T>(T);
#[derive(Clone, Copy)] struct Named<T> { z: (), v: T, p: PhantomData<u64> }
#[derive(Clone, Copy)] struct ZPad<T>([u8; 0], T, ());
#[derive(Clone, Copy)] #[repr(align(4))] struct Aligned(Void);
#[derive(Clone, Copy)] enum AllDead { A(Void), B(Never) }
#[derive(Clone, Copy)] enum DeepDead { A(Wrap<Void>), B((Void,)), C([Void; 1]) }
#[derive(Clone, Copy)] enum HalfDead { A(Void), B }
#[derive(Clone, Copy)] enum HalfDead2 { A, B(Never) }
#[derive(Clone, Copy)] struct NamedDead { a: (), b: Void }
#[derive(Clone, Copy)] #[repr(transparent)] struct TrDead(Void);
#[derive(Clone, Copy)] #[repr(C)] struct CDead(Void);
#[derive(Clone, Copy)] #[repr(packed)] struct PkDead(Void);
#[derive(Clone, Copy)] union UDead { v: Void, u: () }
#[derive(Clone, Copy)] enum SL<T> { Never((), Infallible), Live(T) }
#[derive(Clone, Copy)] enum SL3<T> { D0(Void), D1(Never), Live(T), D3((Void, ())) }

#[derive(Clone, Copy)] #[repr(u8)] enum Small { X = 2, Y = 0xFE }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi { A = 200, B = 253 }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi2 { A = 200, B = 254 }
#[derive(Clone, Copy)] #[repr(u8)] enum Hi3 { A = 250, B = 251, C = 252 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg { A = -3, B = -1 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg2 { A = -56, B = -3 }
#[derive(Clone, Copy)] #[repr(i8)] enum Sg3 { A = 100, B = 125 }
#[derive(Clone, Copy)] #[repr(u16)] enum W16 { A = 0xFFF0, B = 0xFFFD }
#[derive(Clone, Copy)] #[repr(i16)] enum S16 { A = -200, B = -3 }
#[derive(Clone, Copy)] #[repr(u32)] enum W32 { A = 0xFFFF_FF00, B = 0xFFFF_FFFD }
#[derive(Clone, Copy)] #[repr(i32)] enum S32 { A = -70000, B = -3 }
#[derive(Clone, Copy)] #[repr(u64)] enum W64 { A = 0xFFFF_FFFF_FFFF_FF00, B = 0xFFFF_FFFF_FFFF_FFFD }
#[derive(Clone, Copy)] #[repr(i64)] enum S64 { A = -5_000_000_000, B = -3 }
#[derive(Clone, Copy)] #[repr(usize)] enum WU { A = 0xFFFF_FF00, B = 0xFFFF_FFFD }
#[derive(Clone, Copy)] enum Three { A, B, C }
#[derive(Clone, Copy)] enum Two { A, B }

static SEVEN: u32 = 77;
fn seven() -> u32 { 7 }
fn eight() -> u32 { 8 }

macro_rules! suite {
    ($id:expr, $f:ident, $T:ty, [$($k:expr),* $(,)?], [$($r:expr),* $(,)?], |$e:ident| $body:expr) => {
        #[inline(never)]
        fn $f() {
            print_i32(-(1000 + $id));
            // the same values built at run time
            $(
                {
                    #[inline(never)]
                    fn rt_cls($e: $T) -> u32 { $body }
                    #[inline(never)]
                    fn mk() -> $T { $r }
                    { let $e = $r; print_u32($body); }
                    print_u32(rt_cls($r));
                    print_u32(rt_cls(mk()));
                    let o = Some(mk());
                    print_u32(match black_box(o) { Some($e) => $body, None => 55555 });
                    let oo = black_box(Some(Some($r)));
                    print_u32(match oo { Some(Some($e)) => $body, Some(None) => 55556, None => 55557 });
                    print_bool(core::mem::discriminant(&mk()) == core::mem::discriminant(&$r));
                }
            )*
            #[inline(never)]
            fn cls($e: $T) -> u32 { $body }
            #[inline(never)]
            fn cls_ref(r: &$T) -> u32 { let $e = *r; $body }
            #[inline(never)]
            fn cls_o(o: Option<$T>) -> u32 { match o { Some($e) => $body, None => 55555 } }
            #[inline(never)]
            fn cls_oo(o: Option<Option<$T>>) -> u32 { match o { Some(Some($e)) => $body, Some(None) => 55556, None => 55557 } }
            static TABLE: &[$T] = &[$($k),*];
            static OTABLE: [Option<$T>; 2] = [None, None];
            $(
                {
                    const K: $T = $k;
                    static S: $T = $k;
                    { let $e = K; print_u32($body); }
                    print_u32(cls(K));
                    print_u32(cls(black_box(K)));
                    print_u32(cls_ref(&K));
                    print_u32(cls_ref(black_box(&S)));
                    print_u32(cls(S));
                    const O: Option<$T> = Some($k);
                    static SO: Option<$T> = Some($k);
                    print_u32(match O { Some($e) => $body, None => 55555 });
                    print_u32(cls_o(O));
                    print_u32(cls_o(black_box(O)));
                    print_u32(cls_o(SO));
                    const OO: Option<Option<$T>> = Some(Some($k));
                    print_u32(cls_oo(OO));
                    print_u32(cls_oo(black_box(OO)));
                    print_bool(matches!(black_box(OO), Some(Some(_))));
                    print_bool(core::mem::discriminant(&K) == core::mem::discriminant(black_box(&S)));
                }
            )*
            { const N: Option<$T> = None; print_u32(cls_o(N)); print_u32(cls_o(black_box(N))); print_bool(N.is_none()); print_u32(match N { Some($e) => $body, None => 55555 }); }
            { const N: Option<Option<$T>> = Some(None); print_u32(cls_oo(N)); print_u32(cls_oo(black_box(N))); }
            { const N: Option<Option<$T>> = None; print_u32(cls_oo(N)); print_u32(cls_oo(black_box(N))); }
            print_u32(cls_o(OTABLE[black_box(1)]));
            let mut i = 0;
            while i < TABLE.len() {
                print_u32(cls(TABLE[black_box(i)]));
                let mut j = 0;
                while j < TABLE.len() {
                    print_bool(core::mem::discriminant(&TABLE[black_box(i)]) == core::mem::discriminant(&TABLE[j]));
                    j += 1;
                }
                i += 1;
            }
        }
    };
}
'''

# payload: (type, [value exprs], printer taking `p`)
PAYLOADS = [
    ("char", ["'a'", "'\\u{10FFFF}'", "'\\0'"], "p as u32"),
    ("bool", ["true", "false"], "10 + p as u32"),
    ("NonZeroU8", ["NonZeroU8::new(1).unwrap()", "NonZeroU8::new(255).unwrap()"], "p.get() as u32"),
    ("NonZeroI8", ["NonZeroI8::new(-1).unwrap()", "NonZeroI8::new(127).unwrap()"], "p.get() as i32 as u32"),
    ("NonZeroU16", ["NonZeroU16::new(1).unwrap()", "NonZeroU16::new(65535).unwrap()"], "p.get() as u32"),
    ("NonZeroI16", ["NonZeroI16::new(-1).unwrap()", "NonZeroI16::new(300).unwrap()"], "p.get() as i32 as u32"),
    ("NonZeroU32", ["NonZeroU32::new(1).unwrap()", "NonZeroU32::new(0xFFFF_FFFF).unwrap()"], "p.get()"),
    ("NonZeroU64", ["NonZeroU64::new(1).unwrap()", "NonZeroU64::new(0xFFFF_FFFF_FFFF_FFFF).unwrap()", "NonZeroU64::new(0x1_0000_0000).unwrap()"], "(p.get() % 1_000_003) as u32"),
    ("Small", ["Small::X", "Small::Y"], "p as u32"),
    ("Hi", ["Hi::A", "Hi::B"], "p as u32"),
    ("Hi2", ["Hi2::A", "Hi2::B"], "p as u32"),
    ("Hi3", ["Hi3::A", "Hi3::B", "Hi3::C"], "p as u32"),
    ("Sg", ["Sg::A", "Sg::B"], "p as i32 as u32"),
    ("Sg2", ["Sg2::A", "Sg2::B"], "p as i32 as u32"),
    ("Sg3", ["Sg3::A", "Sg3::B"], "p as i32 as u32"),
    ("W16", ["W16::A", "W16::B"], "p as u32"),
    ("S16", ["S16::A", "S16::B"], "p as i32 as u32"),
    ("W32", ["W32::A", "W32::B"], "p as u32"),
    ("S32", ["S32::A", "S32::B"], "p as i32 as u32"),
    ("W64", ["W64::A", "W64::B"], "(p as u64 % 1_000_003) as u32"),
    ("S64", ["S64::A", "S64::B"], "(p as i64 as u64 % 1_000_003) as u32"),
    ("WU", ["WU::A", "WU::B"], "p as usize as u32"),
    ("Three", ["Three::A", "Three::B", "Three::C"], "p as u32 + 20"),
    ("Two", ["Two::A", "Two::B"], "p as u32 + 30"),
    ("Ordering", ["Ordering::Less", "Ordering::Equal", "Ordering::Greater"], "p as i32 as u32"),
    ("&'static u32", ["&5", "&SEVEN"], "*p"),
    ("fn() -> u32", ["seven", "eight"], "p()"),
    ("Option<bool>", ["Some(true)", "Some(false)", "None"], "match p { Some(b) => 40 + b as u32, None => 49 }"),
    ("Option<char>", ["Some('z')", "None"], "match p { Some(c) => c as u32, None => 48 }"),
    ("Option<Hi2>", ["Some(Hi2::A)", "Some(Hi2::B)", "None"], "match p { Some(h) => h as u32, None => 47 }"),
    ("Option<Option<Three>>", ["Some(Some(Three::C))", "Some(None)", "None"], "match p { Some(Some(t)) => t as u32 + 60, Some(None) => 46, None => 45 }"),
]

# wrappers around the payload: (type template, value template, access template on `p` -> inner)
WRAPS = [
    ("{T}", "{v}", "{p}"),
    ("{T}", "{v}", "{p}"),
    ("{T}", "{v}", "{p}"),
    ("Wrap<{T}>", "Wrap({v})", "{p}.0"),
    ("Tr<{T}>", "Tr({v})", "{p}.0"),
    ("CW<{T}>", "CW({v})", "{p}.0"),
    ("Pk<{T}>", "Pk({v})", "{{ {p}.0 }}"),
    ("Named<{T}>", "Named {{ z: (), v: {v}, p: PhantomData }}", "{p}.v"),
    ("ZPad<{T}>", "ZPad([], {v}, ())", "{p}.1"),
    ("((), {T})", "((), {v})", "{p}.1"),
    ("({T},)", "({v},)", "{p}.0"),
    ("({T}, PhantomData<Void>, [Void; 0])", "({v}, PhantomData, [])", "{p}.0"),
    ("[{T}; 1]", "[{v}]", "{p}[0]"),
    ("SL<{T}>", "SL::Live({v})", "(match {p} {{ SL::Live(x) => x, SL::Never(..) => loop {{}} }})"),
    ("SL3<{T}>", "SL3::Live({v})", "(match {p} {{ SL3::Live(x) => x, _ => loop {{}} }})"),
    ("Result<{T}, Void>", "Ok({v})", "(match {p} {{ Ok(x) => x, Err(_) => loop {{}} }})"),
    ("Result<Never, {T}>", "Err({v})", "(match {p} {{ Err(x) => x, Ok(_) => loop {{}} }})"),
    ("ManuallyDrop<{T}>", "ManuallyDrop::new({v})", "ManuallyDrop::into_inner({p})"),
    ("Wrap<Tr<((), {T})>>", "Wrap(Tr(((), {v})))", "{p}.0.0.1"),
    ("SL<Wrap<SL3<{T}>>>", "SL::Live(Wrap(SL3::Live({v})))", "(match {p} {{ SL::Live(Wrap(SL3::Live(x))) => x, _ => loop {{}} }})"),
]

# dead/zst variant payloads: (type list for a tuple variant, value exprs if inhabited else None)
DEADS = [
    (["Void"], None), (["Never"], None), (["Infallible"], None),
    (["(Void,)"], None), (["[Void; 2]"], None), (["Wrap<Void>"], None),
    (["(Void, ())"], None), (["()", "Infallible"], None), (["Void", "()", "PhantomData<u8>"], None),
    (["[u32; 0]", "Void"], None), (["Void", "[u64; 0]"], None), (["u8", "Void"], None),
    (["Aligned"], None), (["AllDead"], None), (["DeepDead"], None), (["NamedDead"], None),
    (["TrDead"], None), (["CDead"], None), (["PkDead"], None), (["ManuallyDrop<Void>"], None),
    (["Wrap<Wrap<(Void, ())>>"], None), (["[[Void; 1]; 1]"], None), (["([Void; 0], Void)"], None),
    (["(Never, Never)"], None), (["Result<Void, Never>"], None), (["[(Void,); 3]"], None),
    (["Wrap<AllDead>"], None), (["(HalfDead, Void)"], None), (["[u16; 0]", "Void"], None),
    (["(u32, Void)"], None), (["&'static Void", "Void"], None),
    # inhabited zero-sized controls
    (["[Void; 0]"], ["[]"]), (["PhantomData<Void>"], ["PhantomData"]), (["()"], ["()"]),
    (["HalfDead"], ["HalfDead::B"]), (["HalfDead2"], ["HalfDead2::A"]), (["Option<Void>"], ["None"]),
    (["Result<(), Void>"], ["Ok(())"]), (["MaybeUninit<Void>"], None),
    (["[(Void,); 0]"], ["[]"]), (["UDead"], None), (["[u32; 0]"], ["[]"]),
    (["Wrap<HalfDead>", "()"], ["Wrap(HalfDead::B)", "()"]),
]


def gen_enum(rng, idx):
    pty, pvals, pprint = rng.choice(PAYLOADS)
    wty, wval, wacc = rng.choice(WRAPS)
    live_ty = wty.format(T=pty)
    name = f"E{idx}"
    n_units = rng.choice([0, 0, 1, 1, 2, 2, 3, 4, 6])
    n_dead = rng.choice([0, 1, 1, 2, 2, 3])
    slots = ["live"] + [f"u{i}" for i in range(n_units)] + [f"d{i}" for i in range(n_dead)]
    rng.shuffle(slots)
    variants, consts, arms, rts = [], [], [], []
    # Known pre-existing crash on main (js-lane=jir js-opt=off): a tagged enum with exactly one
    # shallowly inhabited variant (see src/min/m1_dead_nonzst.rs). Keep it out of this family so it
    # does not mask everything after it in a file.
    EMPTY = ("Void", "Never", "Infallible")
    picks = {}
    while True:
        picks = {s: rng.choice(DEADS) for s in slots if s.startswith("d")}
        shallow_live = 1 + n_units + sum(1 for tys, _ in picks.values() if not any(t in EMPTY for t in tys))
        bad = shallow_live == 1 and any(("u8" in tys or "&'static Void" in tys) for tys, _ in picks.values())
        if not bad:
            break
    for s in slots:
        if s == "live":
            variants.append(f"Live({live_ty})")
            for v in pvals:
                consts.append(f"{name}::Live({wval.format(v=v)})")
                rts.append(f"{name}::Live({wval.format(v='black_box(' + v + ')')})")
            inner = wacc.format(p="w")
            arms.append(f"{name}::Live(w) => {{ let p: {pty} = {inner}; {pprint} }}")
        elif s.startswith("u"):
            i = int(s[1:])
            variants.append(f"U{i}")
            consts.append(f"{name}::U{i}")
            arms.append(f"{name}::U{i} => {1001 + i}")
        else:
            i = int(s[1:])
            tys, vals = picks[s]
            variants.append(f"D{i}({', '.join(tys)})")
            if vals is not None:
                consts.append(f"{name}::D{i}({', '.join(vals)})")
                arms.append(f"{name}::D{i}(..) => {2001 + i}")
            else:
                arms.append(f"{name}::D{i}(..) => {9001 + i}")
    decl = f"#[derive(Clone, Copy)] enum {name} {{ {', '.join(variants)} }}"
    suite = (f"suite!({idx}, s{idx}, {name}, [{', '.join(consts)}], [{', '.join(rts)}], "
             f"|e| match e {{ {', '.join(arms)} }});")
    return decl + "\n" + suite + "\n"


def main():
    out, seed, n, per = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
    os.makedirs(out, exist_ok=True)
    rng = random.Random(seed)
    idx = 0
    fileno = 0
    while idx < n:
        body = HEAD
        calls = []
        for _ in range(per):
            if idx >= n:
                break
            body += gen_enum(rng, idx)
            calls.append(f"s{idx}();")
            idx += 1
        body += "\n#[unsafe(no_mangle)]\npub fn rust_entry() {\n    " + " ".join(calls) + "\n}\n"
        with open(os.path.join(out, f"L{seed}_{fileno:02d}.rs"), "w") as f:
            f.write(body)
        fileno += 1


main()
