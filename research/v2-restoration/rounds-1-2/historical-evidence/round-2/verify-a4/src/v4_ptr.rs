//! Verifier probe v4: pointer niches in constant data, my own family (carriers x places).
#![no_std]
#![no_main]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, Ordering};

// ---- carriers -----------------------------------------------------------------------------
#[derive(Clone, Copy)]
struct Entry {
    key: u8,
    val: Option<&'static u64>,
    name: Option<&'static str>,
    bytes: Option<&'static [u8]>,
    cb: Option<fn(u32) -> u32>,
}
fn dbl(x: u32) -> u32 { x * 2 }
fn inc(x: u32) -> u32 { x + 1 }

static E_STATIC: Entry = Entry { key: 1, val: Some(&11), name: Some("one"), bytes: Some(b"ab"), cb: Some(dbl) };
const E_CONST: Entry = Entry { key: 2, val: None, name: Some(""), bytes: Some(&[]), cb: None };
static TABLE: [Entry; 4] = [
    Entry { key: 3, val: Some(&33), name: None, bytes: None, cb: Some(inc) },
    E_CONST,
    Entry { key: 4, val: None, name: Some("four"), bytes: Some(&[9, 8, 7]), cb: None },
    Entry { key: 5, val: Some(&u64::MAX), name: Some("five"), bytes: Some(b""), cb: Some(dbl) },
];
const SLICE: &[Entry] = &[E_CONST, Entry { key: 6, val: Some(&66), name: Some("six"), bytes: None, cb: Some(inc) }];

#[inline(never)]
fn show(e: &Entry) {
    print_u32(e.key as u32);
    match e.val { Some(v) => print_u64(*v), None => print_str("val:none") }
    match e.name { Some(n) => { print_str(n); print_usize(n.len()) } None => print_str("name:none") }
    match e.bytes { Some(b) => { print_usize(b.len()); if let Some(x) = b.first() { print_u32(*x as u32) } } None => print_str("bytes:none") }
    match e.cb { Some(f) => print_u32(f(20)), None => print_str("cb:none") }
    print_bool(e.val.is_some());
    print_bool(e.name.is_none());
    print_bool(e.bytes.is_some());
    print_bool(e.cb.is_none());
}

// ---- generic and associated positions ---------------------------------------------------
trait Named { const NAME: Option<&'static str>; const DEFAULT: Option<&'static u32> = None; const CB: Option<fn() -> u32>; }
struct Alpha; struct Beta;
fn seven() -> u32 { 7 }
impl Named for Alpha { const NAME: Option<&'static str> = Some("alpha"); const DEFAULT: Option<&'static u32> = Some(&5); const CB: Option<fn() -> u32> = Some(seven); }
impl Named for Beta { const NAME: Option<&'static str> = None; const CB: Option<fn() -> u32> = None; }
#[inline(never)]
fn named<T: Named>() {
    print_str(T::NAME.unwrap_or("anon"));
    print_u32(T::DEFAULT.map_or(0, |v| *v));
    print_u32(T::CB.map_or(1000, |f| f()));
    print_bool(black_box(T::NAME).is_some());
    print_bool(black_box(T::DEFAULT).is_some());
    print_bool(black_box(T::CB).is_some());
    let r: &'static Option<&'static str> = &T::NAME;
    print_bool(r.is_some());
}
struct Gen<T>(T);
impl<T: Named> Gen<T> { const PAIR: (Option<&'static str>, Option<&'static u32>) = (T::NAME, T::DEFAULT); const INNER: Option<Option<&'static str>> = Some(T::NAME); }

// ---- wrappers, nesting, multi-variant ----------------------------------------------------
#[derive(Clone, Copy)]
enum Tri { Ref(&'static u32), Nothing }
#[derive(Clone, Copy)]
enum Wide { Text(&'static str), Missing }
#[derive(Clone, Copy)]
enum Deep { Pair(u32, &'static u32), Zip }
#[derive(Clone, Copy)]
struct Newtype(Option<&'static u32>);
#[derive(Clone, Copy)]
#[repr(transparent)]
struct Transparent(Option<&'static str>);
const TRI: [Tri; 2] = [Tri::Ref(&21), Tri::Nothing];
const TRI_ONE: Tri = Tri::Ref(&22);
const TRI_NONE: Tri = Tri::Nothing;
static WIDE: [Wide; 2] = [Wide::Missing, Wide::Text("wide")];
const WIDE_ONE: Wide = Wide::Text("w1");
static DEEP: [Deep; 2] = [Deep::Pair(1, &2), Deep::Zip];
const DEEP_ONE: Deep = Deep::Pair(3, &4);
const NT: Newtype = Newtype(Some(&31));
const NT_NONE: Newtype = Newtype(None);
static TR: Transparent = Transparent(Some("tr"));
const TR_NONE: Transparent = Transparent(None);
const NESTED: [Option<Option<&'static u32>>; 3] = [Some(Some(&41)), Some(None), None];
const NESTED_ONE: Option<Option<&'static u32>> = Some(Some(&42));
const NESTED_MID: Option<Option<&'static u32>> = Some(None);
const REF_REF: Option<&'static &'static u32> = Some(&&43);
const REF_OPT: Option<&'static Option<&'static u32>> = Some(&Some(&44));
const REF_OPT_NONE: Option<&'static Option<&'static u32>> = Some(&None);
const RES_OK: Result<&'static u32, ()> = Ok(&45);
const RES_ERR: Result<&'static u32, ()> = Err(());
const RES2_OK: Result<(), &'static str> = Ok(());
const RES2_ERR: Result<(), &'static str> = Err("bad");
const UNIT_REF: Option<&'static ()> = Some(&());
const EMPTY_ARR: Option<&'static [u32; 0]> = Some(&[]);
const EMPTY_SLICE: Option<&'static [u32]> = Some(&[]);
const EMPTY_STR: Option<&'static str> = Some("");
const TUPLE: (u8, Option<&'static u32>, u8) = (1, Some(&46), 2);
const TUPLE_NONE: (u8, Option<&'static u32>, u8) = (3, None, 4);
const ARR_REF: Option<&'static [u32; 3]> = Some(&[5, 6, 7]);
const ARR_ELEM: Option<&'static u32> = Some(&[50, 60, 70][1]);
const STRUCT_FIELD: Option<&'static u32> = Some(&(1u32, 2u32).1);

#[inline(never)] fn tri(t: Tri) -> u32 { match t { Tri::Ref(r) => *r, Tri::Nothing => 0 } }
#[inline(never)] fn wide(w: Wide) -> usize { match w { Wide::Text(s) => s.len(), Wide::Missing => 99 } }
#[inline(never)] fn deep(d: Deep) -> u32 { match d { Deep::Pair(a, b) => a * 10 + *b, Deep::Zip => 0 } }
#[inline(never)] fn nested(n: Option<Option<&'static u32>>) -> u32 { match n { Some(Some(v)) => *v, Some(None) => 1, None => 0 } }

// ---- statics naming statics ---------------------------------------------------------------
static COUNTER: AtomicU32 = AtomicU32::new(5);
static COUNTER_REF: Option<&AtomicU32> = Some(&COUNTER);
static BASE: u32 = 900;
static BASE_REF: Option<&u32> = Some(&BASE);
static BASE_REF2: Option<&u32> = Some(&BASE);
static BASE_REF_REF: Option<&Option<&u32>> = Some(&BASE_REF);
struct Registry { first: Option<&'static Entry>, second: Option<&'static Entry>, all: Option<&'static [Entry; 4]> }
static REGISTRY: Registry = Registry { first: Some(&E_STATIC), second: None, all: Some(&TABLE) };
static mut SLOT: Option<&'static u32> = Some(&BASE);
static mut SLOT_FN: Option<fn(u32) -> u32> = None;

// ---- NonNull ------------------------------------------------------------------------------
struct SyncPtr(Option<NonNull<u32>>);
unsafe impl Sync for SyncPtr {}
static NN_DANGLING: SyncPtr = SyncPtr(Some(NonNull::dangling()));
static NN_NONE: SyncPtr = SyncPtr(None);
const NN_CONST: Option<NonNull<u8>> = Some(NonNull::dangling());

#[unsafe(no_mangle)]
pub fn rust_entry() {
    show(&E_STATIC);
    show(&E_CONST);
    let mut i = 0;
    while i < TABLE.len() { show(&TABLE[black_box(i)]); i += 1; }
    for e in SLICE { show(e); }
    let local = Entry { key: 9, val: Some(&99), name: Some("local"), bytes: Some(&[1]), cb: Some(inc) };
    show(black_box(&local));
    let promoted: &'static Entry = &Entry { key: 10, val: Some(&100), name: None, bytes: Some(&[2, 3]), cb: None };
    show(promoted);
    let promoted_opt: &'static Option<&'static str> = &Some("promoted");
    print_str(promoted_opt.unwrap_or("none"));

    named::<Alpha>();
    named::<Beta>();
    print_str(Gen::<Alpha>::PAIR.0.unwrap_or("anon"));
    print_u32(Gen::<Alpha>::PAIR.1.map_or(0, |v| *v));
    print_str(Gen::<Beta>::PAIR.0.unwrap_or("anon"));
    print_u32(Gen::<Beta>::PAIR.1.map_or(0, |v| *v));
    print_str(match Gen::<Alpha>::INNER { Some(Some(s)) => s, Some(None) => "inner-none", None => "none" });
    print_str(match Gen::<Beta>::INNER { Some(Some(s)) => s, Some(None) => "inner-none", None => "none" });

    print_u32(tri(TRI[black_box(0)])); print_u32(tri(TRI[black_box(1)]));
    print_u32(tri(TRI_ONE)); print_u32(tri(black_box(TRI_ONE))); print_u32(tri(TRI_NONE));
    print_u32(match TRI_ONE { Tri::Ref(r) => *r, Tri::Nothing => 0 });
    print_u32(match TRI_NONE { Tri::Ref(r) => *r, Tri::Nothing => 0 });
    print_usize(wide(WIDE[black_box(0)])); print_usize(wide(WIDE[black_box(1)])); print_usize(wide(WIDE_ONE));
    print_usize(match WIDE_ONE { Wide::Text(s) => s.len(), Wide::Missing => 99 });
    print_u32(deep(DEEP[black_box(0)])); print_u32(deep(DEEP[black_box(1)])); print_u32(deep(DEEP_ONE));
    print_u32(NT.0.map_or(0, |v| *v)); print_bool(NT_NONE.0.is_none()); print_bool(black_box(NT).0.is_some());
    print_str(TR.0.unwrap_or("none")); print_bool(TR_NONE.0.is_none()); print_bool(black_box(&TR).0.is_some());
    print_u32(nested(NESTED[black_box(0)])); print_u32(nested(NESTED[black_box(1)])); print_u32(nested(NESTED[black_box(2)]));
    print_u32(nested(NESTED_ONE)); print_u32(nested(NESTED_MID)); print_u32(nested(black_box(NESTED_ONE)));
    print_u32(match NESTED_ONE { Some(Some(v)) => *v, Some(None) => 1, None => 0 });
    print_u32(match NESTED_MID { Some(Some(v)) => *v, Some(None) => 1, None => 0 });
    print_u32(REF_REF.map_or(0, |v| **v));
    print_u32(REF_OPT.map_or(0, |v| v.map_or(1, |w| *w)));
    print_u32(REF_OPT_NONE.map_or(0, |v| v.map_or(1, |w| *w)));
    print_u32(black_box(REF_OPT).map_or(0, |v| v.map_or(1, |w| *w)));
    print_u32(RES_OK.map_or(0, |v| *v)); print_bool(RES_ERR.is_err()); print_bool(black_box(RES_OK).is_ok());
    print_bool(RES2_OK.is_ok()); print_str(RES2_ERR.err().unwrap_or("ok")); print_bool(black_box(RES2_ERR).is_err());
    print_bool(UNIT_REF.is_some()); print_bool(black_box(UNIT_REF).is_some());
    print_bool(EMPTY_ARR.is_some()); print_usize(EMPTY_ARR.map_or(9, |a| a.len()));
    print_bool(EMPTY_SLICE.is_some()); print_usize(black_box(EMPTY_SLICE).map_or(9, |a| a.len()));
    print_bool(EMPTY_STR.is_some()); print_usize(black_box(EMPTY_STR).map_or(9, |a| a.len()));
    print_u32(TUPLE.1.map_or(0, |v| *v) + TUPLE.0 as u32 + TUPLE.2 as u32);
    print_u32(black_box(TUPLE_NONE).1.map_or(0, |v| *v) + TUPLE_NONE.0 as u32 + TUPLE_NONE.2 as u32);
    print_u32(ARR_REF.map_or(0, |a| a[2])); print_u32(black_box(ARR_REF).map_or(0, |a| a[0]));
    print_u32(ARR_ELEM.map_or(0, |v| *v)); print_u32(black_box(ARR_ELEM).map_or(0, |v| *v));
    print_u32(STRUCT_FIELD.map_or(0, |v| *v)); print_u32(black_box(STRUCT_FIELD).map_or(0, |v| *v));

    print_u32(COUNTER_REF.map_or(0, |c| c.load(Ordering::Relaxed)));
    COUNTER.store(6, Ordering::Relaxed);
    print_u32(black_box(&COUNTER_REF).map_or(0, |c| c.load(Ordering::Relaxed)));
    COUNTER_REF.unwrap().store(8, Ordering::Relaxed);
    print_u32(COUNTER.load(Ordering::Relaxed));
    print_bool(core::ptr::eq(COUNTER_REF.unwrap(), &COUNTER));
    print_u32(BASE_REF.map_or(0, |v| *v));
    print_bool(core::ptr::eq(BASE_REF.unwrap(), BASE_REF2.unwrap()));
    print_bool(core::ptr::eq(BASE_REF.unwrap(), &BASE));
    print_u32(BASE_REF_REF.map_or(0, |v| v.map_or(1, |w| *w)));
    print_bool(core::ptr::eq(BASE_REF_REF.unwrap(), &BASE_REF));
    print_u32(REGISTRY.first.map_or(0, |e| e.key as u32));
    print_bool(REGISTRY.second.is_none());
    print_u32(black_box(&REGISTRY).all.map_or(0, |t| t[3].key as u32));
    print_bool(core::ptr::eq(REGISTRY.first.unwrap(), &E_STATIC));
    print_bool(core::ptr::eq(REGISTRY.all.unwrap(), &TABLE));
    unsafe {
        let slot = &raw mut SLOT;
        print_u32((*slot).map_or(0, |v| *v));
        print_bool(core::ptr::eq((*slot).unwrap(), &BASE));
        *slot = None;
        print_bool((*slot).is_none());
        *slot = Some(&7);
        print_u32((*slot).map_or(0, |v| *v));
        let slot_fn = &raw mut SLOT_FN;
        print_bool((*slot_fn).is_none());
        *slot_fn = Some(dbl);
        print_u32((*slot_fn).map_or(0, |f| f(21)));
    }
    print_bool(NN_DANGLING.0.is_some()); print_bool(black_box(&NN_DANGLING).0.is_some());
    print_bool(NN_NONE.0.is_none()); print_bool(NN_CONST.is_some()); print_bool(black_box(NN_CONST).is_some());
}
