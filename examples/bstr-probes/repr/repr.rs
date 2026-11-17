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

extern crate alloc;

use alloc::format;
use alloc::string::String;
use mires_js::{js_bindings, JsValue};

unsafe extern "C-unwind" {
    fn js_log_str(s: &str);
    fn js_abort(msg: &str) -> !;
    fn bstr_ext_pair(value: (&str, u32));
    fn bstr_ext_opt() -> Option<&'static str>;
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
pub enum Shape {
    Text(&'static str),
    Pair {
        left: &'static str,
        right: &'static str,
    },
    Count(u32),
    Empty,
}

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
