//! The `js-str` host-representation probes (`scripts/bstr-probes.sh`): a `str` nested in a value
//! the host sees, in both directions and on every route — `js_bindings!` parameters and results
//! (plain, nullable, fallible), `js!` captures and results, `extern "C"` imports, and exports.
//!
//! The host prints every value it is handed as JSON (`repr.js`), so the probe compares the
//! JavaScript *representation* the host observes: `js-str=bytes` must print exactly what
//! `js-str=utf16` (main) prints. Values the host hands back are measured and echoed by Rust, so
//! a missed encode shows as a wrong byte length.

#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;
use core::mem::MaybeUninit;
use mires_js::{js_bindings, JsValue};

unsafe extern "C-unwind" {
    fn js_log_str(s: &str);
    fn js_abort(msg: &str) -> !;
    fn bstr_ext_pair(value: (&str, u32));
    fn bstr_ext_opt() -> Option<&'static str>;
    fn bstr_ext_reflect_mut(mode: u32, value: &mut SchemaA);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { js_abort("panic") }
}

fn say(line: &str) {
    unsafe { js_log_str(line) }
}

#[derive(Clone, Copy)]
pub struct Named {
    pub name: &'static str,
    pub n: u32,
}

#[derive(Clone, Copy)]
pub struct SchemaA {
    pub s: &'static str,
    pub n: u32,
}

#[derive(Clone, Copy)]
pub struct SchemaB {
    pub s: &'static str,
    pub n: u32,
    pub t: &'static str,
}

/// Deliberate `Location` struct name collision with `core::panic::Location`.
pub struct Location<'a> {
    pub name: &'a str,
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy)]
pub struct WithZstArr {
    pub label: &'static str,
    pub none: [&'static str; 0],
}

pub struct WithVec {
    pub items: Vec<&'static str>,
    pub tag: &'static str,
}

pub struct Cyc<'a> {
    pub label: &'static str,
    pub next: Cell<Option<&'a Cyc<'a>>>,
}

type Deep4 = Option<Option<Option<Option<&'static str>>>>;
type Deep16 = Option<Option<Option<Option<Option<Option<Option<Option<Option<Option<Option<Option<Deep4>>>>>>>>>>>>;

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.callback0")]
#[inline(never)]
fn typed_cb0<F>(f: F) -> JsValue
where
    F: Fn() -> &'static str,
{
    let _ = f();
    core::mem::forget(f);
    JsValue::undefined()
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.callback1")]
#[inline(never)]
fn typed_cb1<F>(f: F) -> JsValue
where
    F: Fn(&'static str) -> (&'static str, u32),
{
    let _ = f("");
    core::mem::forget(f);
    JsValue::undefined()
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.callback2")]
#[inline(never)]
fn typed_cb2<F>(f: F) -> JsValue
where
    F: Fn(&'static str, (&'static str, u32)) -> &'static str,
{
    let _ = f("", ("", 0));
    core::mem::forget(f);
    JsValue::undefined()
}

fn fdef_echo(s: &'static str) -> (&'static str, u32) {
    (s, s.len() as u32)
}

#[derive(Clone, Copy)]
pub enum Shape {
    Text(&'static str),
    Pair {
        left: &'static str,
        right: &'static str,
    },
    Count(u32),
    Empty,
}

type HetTup = (&'static str, Option<&'static str>, JsValue, u32);

js_bindings! {
    #[js(call = "globalThis.bstrSee")]
    fn see_opt(label: &str, value: Option<&str>);
    #[js(call = "globalThis.bstrSee")]
    fn see_pair(label: &str, value: (&str, u32));
    #[js(call = "globalThis.bstrSee")]
    fn see_array(label: &str, value: [&str; 2]);
    #[js(call = "globalThis.bstrSee")]
    fn see_named(label: &str, value: Named);
    #[js(call = "globalThis.bstrSee")]
    fn see_shape(label: &str, value: Shape);
    #[js(call = "globalThis.bstrSee")]
    fn see_nested(label: &str, value: Option<(&str, Named)>);
    #[js(call = "globalThis.bstrSee")]
    fn see_ref_pair(label: &str, value: &(&str, u32));
    #[js(call = "globalThis.bstrSee")]
    fn see_ref_ref(label: &str, value: &&str);
    #[js(call = "globalThis.bstrSee")]
    fn see_slice(label: &str, value: &[&str]);
    #[js(call = "globalThis.bstrSee")]
    fn see_value(label: &str, value: (JsValue, &str));

    // C1 containers, smart pointers, ZST & deep16.
    #[js(call = "globalThis.bstrSee")]
    fn see_vec(label: &str, value: Vec<&'static str>);
    #[js(call = "globalThis.bstrSee")]
    fn see_vec_ref(label: &str, value: &Vec<&'static str>);
    #[js(call = "globalThis.bstrSee")]
    fn see_with_vec(label: &str, value: &WithVec);
    #[js(call = "globalThis.bstrSee")]
    fn see_vec_opt(label: &str, value: Vec<Option<&'static str>>);
    #[js(call = "globalThis.bstrSee")]
    fn see_vecdeque(label: &str, value: &VecDeque<&'static str>);
    #[js(call = "globalThis.bstrSee")]
    fn see_box_pair(label: &str, value: Box<(&'static str, u32)>);
    #[js(call = "globalThis.bstrSee")]
    fn see_box_slice(label: &str, value: Box<[&'static str]>);
    #[js(call = "globalThis.bstrSee")]
    fn see_rc_str(label: &str, value: Rc<&'static str>);
    #[js(call = "globalThis.bstrSee")]
    fn see_rc_pair(label: &str, value: Rc<(&'static str, u32)>);
    #[js(call = "globalThis.bstrSee")]
    fn see_rc_slice(label: &str, value: Rc<[&'static str]>);
    #[js(call = "globalThis.bstrSee")]
    fn see_arc_str(label: &str, value: Arc<&'static str>);
    #[js(call = "globalThis.bstrSee")]
    fn see_btree(label: &str, value: &BTreeMap<&'static str, &'static str>);
    #[js(call = "globalThis.bstrCheckCyc")]
    fn see_cyc(label: &str, value: &Cyc<'_>);
    #[js(call = "globalThis.bstrSee")]
    fn see_zst_arr_param(label: &str, value: [&'static str; 0]);
    #[js(call = "globalThis.bstrSee")]
    fn see_zst_arr_struct(label: &str, value: WithZstArr);
    #[js(call = "globalThis.bstrSee")]
    fn see_deep16(label: &str, value: Deep16);

    // C3 identity, live reads, &mut write-back, Cell interior mutability.
    #[js(call = "globalThis.bstrKeep")]
    fn keep_pair(label: &str, value: &(&'static str, u32)) -> bool;
    #[js(call = "globalThis.bstrKeep")]
    fn keep_named(label: &str, value: &Named) -> bool;
    #[js(call = "globalThis.bstrKeep")]
    fn keep_opt(label: &str, value: &Option<&'static str>) -> bool;
    #[js(call = "globalThis.bstrKeep")]
    fn keep_cell(label: &str, value: &Cell<&'static str>) -> bool;
    #[js(call = "globalThis.bstrReadKept")]
    fn read_kept(label: &str) -> JsValue;
    #[js(call = "globalThis.bstrMutPair")]
    fn mut_pair(value: &mut (&'static str, u32));
    #[js(call = "globalThis.bstrMutNamed")]
    fn mut_named(value: &mut Named);
    #[js(call = "globalThis.bstrMutOpt")]
    fn mut_opt(value: &mut Option<&'static str>);
    #[js(call = "globalThis.bstrMutCell")]
    fn mut_cell(value: &Cell<&'static str>);
    #[js(call = "globalThis.bstrCall")]
    fn host_call_cb(label: &str, cb: JsValue, arg: &str);
    #[js(call = "globalThis.bstrCall0")]
    fn host_call_cb0(label: &str, cb: JsValue);
    #[js(call = "globalThis.bstrCall2")]
    fn host_call_cb2(label: &str, cb: JsValue, a: &str, b: (&str, u32));
    #[js(call = "globalThis.bstrSeeSlot")]
    fn see_het_slot(label: &str, value: &&'static str) -> JsValue;
    #[js(call = "globalThis.bstrSee")]
    fn see_het_tup(label: &str, value: &HetTup);
    #[js(call = "globalThis.bstrMutHetTup")]
    fn mut_het_tup(value: &mut HetTup);

    #[js(call = "globalThis.bstrGetOpt")]
    fn get_opt(which: u32) -> Option<&'static str>;
    #[js(call = "globalThis.bstrGetPair")]
    fn get_pair() -> (&'static str, u32);
    #[js(call = "globalThis.bstrGetNamed")]
    fn get_named() -> Named;
    #[js(nullable, call = "globalThis.bstrMaybe")]
    fn maybe(which: u32) -> Option<&'static str>;
    #[js(fallible, call = "globalThis.bstrTry")]
    fn attempt(which: u32) -> Result<&'static str, JsValue>;

    #[js(call = "globalThis.bstrReflectMut")]
    fn reflect_mut(mode: u32, value: &mut SchemaA);
    #[js(call = "globalThis.bstrReflectArr")]
    fn reflect_arr(mode: u32, value: &mut (&'static str, u32));
}

#[inline(never)]
fn pick(n: u32) -> Option<&'static str> {
    if n == 0 {
        None
    } else {
        Some("é😀")
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Rust to host, through binding parameters.
    see_opt("opt none", pick(0));
    see_opt("opt some", pick(1));
    see_pair("pair", ("é😀", 7));
    see_array("array", ["中", "ü"]);
    see_named(
        "named",
        Named {
            name: "naïve",
            n: 3,
        },
    );
    see_shape("shape text", Shape::Text("Ω"));
    see_shape(
        "shape pair",
        Shape::Pair {
            left: "é",
            right: "😀",
        },
    );
    see_shape("shape count", Shape::Count(9));
    see_shape("shape empty", Shape::Empty);
    see_nested(
        "nested",
        Some((
            "ß",
            Named {
                name: "日本", n: 1
            },
        )),
    );
    let pair = ("ref é", 2);
    see_ref_pair("ref pair", &pair);
    let text = "ref ref ü";
    see_ref_ref("ref ref", &text);
    let list = ["a", "é", "😀"];
    see_slice("slice", &list[1..]);
    see_value("with host value", (JsValue::erase("hôst"), "rüst"));

    // Host to Rust, through binding results.
    for which in 0..2 {
        match get_opt(which) {
            Some(s) => say(&format!("get_opt {which} Some {} {s}", s.len())),
            None => say(&format!("get_opt {which} None")),
        }
        match maybe(which) {
            Some(s) => say(&format!("maybe {which} Some {} {s}", s.len())),
            None => say(&format!("maybe {which} None")),
        }
        match attempt(which) {
            Ok(s) => say(&format!("attempt {which} Ok {} {s}", s.len())),
            Err(_) => say(&format!("attempt {which} Err")),
        }
    }
    let (s, n) = get_pair();
    say(&format!("get_pair {} {s} {n}", s.len()));
    let named = get_named();
    say(&format!(
        "get_named {} {} {}",
        named.name.len(),
        named.name,
        named.n
    ));

    // `js!` captures and results.
    let o = pick(1);
    let seen: JsValue = js_macro::js! { globalThis.bstrSee("js opt", o) };
    let _ = seen;
    let t = ("é😀", 5u32);
    let _: JsValue = js_macro::js! { globalThis.bstrSee("js pair", t) };
    let named = Named { name: "çà", n: 4 };
    let _: JsValue = js_macro::js! { globalThis.bstrSee("js named", named) };
    let r: Option<&str> = js_macro::js! { ({ TAG: "Some", _0: "résumé" }) };
    match r {
        Some(s) => say(&format!("js result Some {} {s}", s.len())),
        None => say("js result None"),
    }
    let p: (&str, u32) = js_macro::js! { ["zwölf", 12] };
    say(&format!("js result pair {} {} {}", p.0.len(), p.0, p.1));

    // `extern "C"` imports, both directions.
    unsafe { bstr_ext_pair(("ëxt", 1)) };
    match unsafe { bstr_ext_opt() } {
        Some(s) => say(&format!("ext_opt Some {} {s}", s.len())),
        None => say("ext_opt None"),
    }

    // C1 containers, smart pointers, ZSTs, MaybeUninit, deep16.
    see_vec("vec", vec!["é", "ü"]);
    let vref = vec!["α", "😀"];
    see_vec_ref("vecref", &vref);
    let wv = WithVec {
        items: vec!["x", "ñ"],
        tag: "täg",
    };
    see_with_vec("withvec", &wv);
    see_vec_opt("vecopt", vec![Some("é"), None, Some("😀")]);
    let mut vd = VecDeque::new();
    vd.push_back("drop");
    vd.push_back("twö");
    vd.pop_front();
    vd.push_back("thrëe");
    vd.push_back("😀");
    see_vecdeque("vecdeque", &vd);
    see_box_pair("boxpair", Box::new(("böx", 42)));
    see_box_slice("boxslice", vec!["b1é", "b2ü"].into_boxed_slice());
    see_rc_str("rcstr", Rc::from("rcé"));
    see_rc_pair("rcpair", Rc::new(("rcp", 7)));
    let rc_sl: Rc<[&'static str]> = Rc::from(["sl0", "sl1é"].as_slice());
    see_rc_slice("rcslice", rc_sl);
    see_arc_str("arcstr", Arc::from("arc😀"));
    let btm_empty: BTreeMap<&'static str, &'static str> = BTreeMap::new();
    see_btree("btm-empty", &btm_empty);
    let cyc = Cyc {
        label: "cyc0é",
        next: Cell::new(None),
    };
    cyc.next.set(Some(&cyc));
    see_cyc("cyc-self", &cyc);

    see_zst_arr_param("zst-arr-param", []);
    see_zst_arr_struct(
        "zst-arr-struct",
        WithZstArr {
            label: "zst-ok é",
            none: [],
        },
    );
    let mu_init: MaybeUninit<(&'static str, u32)> = MaybeUninit::new(("mué", 9));
    let mu_uninit: MaybeUninit<(&'static str, u32)> = MaybeUninit::uninit();
    let _: JsValue = js_macro::js! { globalThis.bstrSee("mu-init", mu_init) };
    let _: JsValue = js_macro::js! { globalThis.bstrSee("mu-uninit", mu_uninit) };
    let d16: Deep16 = Some(Some(Some(Some(Some(Some(Some(Some(Some(Some(Some(Some(
        Some(Some(Some(Some("d16😀")))),
    ))))))))))));
    see_deep16("deep16", d16);

    // C2 closures in js! and callbacks taking/returning str.
    let cap = "cloé";
    let clo = move || cap.len();
    let _: JsValue = js_macro::js! { globalThis.bstrSee("js-closure", globalThis.JSON.stringify(clo)) };
    let _ = clo();
    let prefix = "pfx";
    let tcb = typed_cb1(move |s: &'static str| {
        (if s == "hé" { "ok😀" } else { prefix }, s.len() as u32)
    });
    host_call_cb("typed-cb-hit", tcb, "hé");
    host_call_cb("typed-cb-miss", tcb, "no");
    let tcb0 = typed_cb0(move || prefix);
    host_call_cb0("typed-cb0", tcb0);
    let tcb2 = typed_cb2(move |a: &'static str, b: (&'static str, u32)| {
        if a == "ä" && b.0 == "ö" && b.1 == 7 { "cb2😀" } else { prefix }
    });
    host_call_cb2("typed-cb2", tcb2, "ä", ("ö", 7));
    let fcb = typed_cb1(fdef_echo);
    host_call_cb("fdef-cb", fcb, "fnitemé");

    // C3 object identity, WeakMap/Set, Array.isArray, Object.keys, live reads, &mut + Cell write-back.
    let mut id_pair = ("id0é", 1u32);
    let s1 = keep_pair("pair", &id_pair);
    let s2 = keep_pair("pair", &id_pair);
    say(&format!("pair-same {s1} {s2}"));
    id_pair.0 = "id1😀";
    let _: JsValue = read_kept("pair");
    mut_pair(&mut id_pair);
    say(&format!("after-mut-pair {} {}", id_pair.0.len(), id_pair.0));

    let mut id_named = Named { name: "n0é", n: 2 };
    let n1 = keep_named("named", &id_named);
    let n2 = keep_named("named", &id_named);
    say(&format!("named-same {n1} {n2}"));
    id_named.name = "n1ü";
    let _: JsValue = read_kept("named");
    mut_named(&mut id_named);
    say(&format!(
        "after-mut-named {} {} {}",
        id_named.name.len(),
        id_named.name,
        id_named.n
    ));

    let mut id_opt = Some("o0é");
    let o1 = keep_opt("opt", &id_opt);
    let o2 = keep_opt("opt", &id_opt);
    say(&format!("opt-same {o1} {o2}"));
    id_opt = Some("o1😀");
    let _: JsValue = read_kept("opt");
    mut_opt(&mut id_opt);
    say(&format!(
        "after-mut-opt {:?}",
        id_opt.map(|s| (s.len(), s))
    ));

    let cell = Cell::new("c0é");
    let c1 = keep_cell("cell", &cell);
    let c2 = keep_cell("cell", &cell);
    say(&format!("cell-same {c1} {c2}"));
    cell.set("c1ñ");
    let _: JsValue = read_kept("cell");
    mut_cell(&cell);
    say(&format!("after-mut-cell {} {}", cell.get().len(), cell.get()));

    // Heterogeneous tuple slot borrowing (`&tup.0`) must not corrupt siblings or poison `_bstr_out_ref`.
    let mut het: HetTup = ("é", Some("😀"), JsValue::erase("hôst"), 42);
    let leaked_slot_buf = see_het_slot("het_slot", &het.0);
    see_het_tup("het_tup", &het);
    mut_het_tup(&mut het);
    say(&format!(
        "after-mut-het {} {:?} {}",
        het.0.len(),
        het.1.map(|s| (s.len(), s)),
        het.3
    ));
    see_het_tup("het_tup_after_mut", &het);
    let _: JsValue = see_het_slot("het_slot_after_tup", &het.0);
    say(&format!("leaked_slot_buf_len {}", bstr_exp_het_in_out(&leaked_slot_buf)));

    // Reflection, freeze, accessor, and prototype-child checks via safe `js_bindings!` and `extern "C-unwind"`.
    for mode in 0..9u32 {
        let mut s_bind = SchemaA { s: "é", n: 7 };
        reflect_mut(mode, &mut s_bind);
        let len_bind = s_bind.s.len();
        if mode == 7 {
            s_bind.s = "ü";
        }
        let mut s_arr = ("é", 7u32);
        reflect_arr(mode, &mut s_arr);
        let len_arr = s_arr.0.len();
        if mode == 7 {
            s_arr.0 = "ü";
        }
        let mut s_ext = SchemaA { s: "é", n: 7 };
        unsafe { bstr_ext_reflect_mut(mode, &mut s_ext) };
        let len_ext = s_ext.s.len();
        if mode == 7 {
            s_ext.s = "ü";
        }
        say(&format!(
            "reflect-rust mode={mode} bind={len_bind}/{} arr={len_arr}/{} ext={len_ext}/{}",
            s_bind.s.len(),
            s_arr.0.len(),
            s_ext.s.len()
        ));
    }

    // `#[track_caller]` exported functions (including `Location` struct name collision) called from Rust.
    say(&format!(
        "tc_from_rust {} {}",
        tc_exported("é"),
        tc_exported_file("é")
    ));
    let u_loc = Location { name: "é", x: 3, y: 4 };
    let (tf, tl, tc, tu) = call_tc_ambig_from_rust(&u_loc);
    say(&format!("tc_ambig_from_rust {} {} {} {}", tf, tl, tc, tu));
}

#[inline(never)]
fn call_tc_ambig_from_rust(u: &Location<'_>) -> (&'static str, u32, u32, usize) {
    tc_ambig_exported(u)
}

#[inline(never)]
fn bstr_exp_het_in_out(_v: &JsValue) -> usize {
    1
}

#[unsafe(no_mangle)]
pub fn bstr_exp_in_slot(r: &&'static str) -> usize {
    r.len()
}

#[unsafe(no_mangle)]
pub fn bstr_exp_in_tup(t: &(&'static str, Option<&'static str>, u32)) -> usize {
    t.0.len() + t.1.map_or(0, |s| s.len()) + t.2 as usize
}

#[unsafe(no_mangle)]
pub fn bstr_exp_in_het4(t: &HetTup) -> usize {
    t.0.len() + t.1.map_or(0, |s| s.len()) + t.3 as usize
}

#[unsafe(no_mangle)]
pub fn bstr_exp_vec(n: u32) -> Vec<&'static str> {
    vec!["é", if n == 0 { "ü" } else { "😀" }]
}

#[unsafe(no_mangle)]
pub fn bstr_exp_id(value: &Named) -> &Named {
    value
}

#[unsafe(no_mangle)]
pub fn bstr_exp_mut_pair(value: &mut (&'static str, u32)) -> u32 {
    let old = value.0.len() as u32;
    value.0 = "wrote😀";
    value.1 += 10;
    old
}

#[unsafe(no_mangle)]
pub fn bstr_exp_btree(value: &BTreeMap<&'static str, &'static str>) -> usize {
    value.get("k1é").map_or(value.len(), |v| v.len())
}

#[unsafe(no_mangle)]
pub fn bstr_exp_cyc<'a>(value: &'a Cyc<'a>) -> &'a Cyc<'a> {
    match value.next.get() {
        Some(next) => next,
        None => value,
    }
}

/// Exports, both directions: the host calls these with representation objects.
#[unsafe(no_mangle)]
pub fn bstr_exp_opt(value: Option<&'static str>) -> Option<&'static str> {
    match value {
        Some(s) if s.len() > 2 => Some(&s[2..]),
        other => other,
    }
}

#[unsafe(no_mangle)]
pub fn bstr_exp_pair(value: (&'static str, u32)) -> (usize, &'static str) {
    (value.0.len() + value.1 as usize, value.0)
}

#[unsafe(no_mangle)]
pub fn bstr_exp_named(value: Named) -> Named {
    Named {
        name: value.name,
        n: value.name.len() as u32,
    }
}

#[unsafe(no_mangle)]
pub fn bstr_exp_shape(value: Shape) -> usize {
    match value {
        Shape::Text(s) => s.len(),
        Shape::Pair { left, right } => left.len() * 100 + right.len(),
        Shape::Count(n) => n as usize,
        Shape::Empty => 0,
    }
}

#[unsafe(no_mangle)]
pub fn bstr_exp_owned(value: &str) -> usize {
    let owned = String::from(value);
    owned.len()
}

#[unsafe(no_mangle)]
pub fn bstr_exp_a_len(v: &SchemaA) -> usize {
    v.s.len()
}

#[unsafe(no_mangle)]
pub fn bstr_exp_b_len(v: &SchemaB) -> usize {
    v.t.len()
}

#[unsafe(no_mangle)]
pub fn bstr_exp_a_identity(v: &SchemaA) -> &SchemaA {
    v
}

#[unsafe(no_mangle)]
pub fn bstr_exp_a_same(a: &SchemaA, b: &SchemaA) -> bool {
    core::ptr::eq(a, b)
}

#[unsafe(no_mangle)]
pub fn bstr_exp_reflect_export(mode: u32, v: &mut SchemaA) -> usize {
    reflect_mut(mode, v);
    let len = v.s.len();
    if mode == 7 {
        v.s = "ü";
    }
    len
}

#[track_caller]
#[unsafe(no_mangle)]
pub fn tc_exported(s: &str) -> usize {
    let loc = core::panic::Location::caller();
    s.len() + loc.file().len() + loc.line() as usize + loc.column() as usize
}

#[track_caller]
#[unsafe(no_mangle)]
pub fn tc_exported_file(s: &str) -> &'static str {
    let _ = s;
    core::panic::Location::caller().file()
}

#[track_caller]
#[unsafe(no_mangle)]
pub fn tc_ambig_exported(u: &Location<'_>) -> (&'static str, u32, u32, usize) {
    let loc = core::panic::Location::caller();
    (loc.file(), loc.line(), loc.column(), u.name.len() + (u.x + u.y) as usize)
}
