#![no_std]
#![no_main]
#![deny(unsafe_code)]

use mires_js::{JsValue, js_bindings, js_export, throw_value};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("explorer panicked"))
}

js_bindings! {
    #[js(call = "Reflect.get")] fn property(target: JsValue, key: JsValue) -> JsValue;
    #[js(call = "Reflect.set")] fn assign(target: JsValue, key: JsValue, value: JsValue) -> bool;
    #[js(new = "Array")] fn array() -> JsValue;
    #[js(new = "Object")] fn object() -> JsValue;
    #[js(index)] fn at(target: JsValue, index: usize) -> JsValue;
    #[js(get = "length")] fn len(target: JsValue) -> usize;
    #[js(method = "push")] fn push(target: JsValue, value: JsValue) -> usize;
    #[js(method = "join")] fn join(target: JsValue, separator: JsValue) -> JsValue;
    #[js(call = "JSON.stringify")] fn stringify(value: JsValue) -> JsValue;
    #[js(call = "JSON.parse")] fn parse(value: JsValue) -> JsValue;
}
fn s(x: &str) -> JsValue {
    JsValue::erase(x)
}
fn get(o: JsValue, k: &str) -> JsValue {
    property(o, s(k))
}
fn obj(fields: &[(&str, JsValue)]) -> JsValue {
    let o = object();
    for &(k, v) in fields {
        let _ = assign(o, s(k), v);
    }
    o
}
fn list(items: &[JsValue]) -> JsValue {
    let a = array();
    for &v in items {
        push(a, v);
    }
    a
}
/// One idb script (idb CONTRACT.md, "Input"): create the store, put every row in one
/// readwrite transaction, then walk the store with a cursor in primary-key order.
fn persist(rows: JsValue) -> JsValue {
    let schema = list(&[
        obj(&[
            ("op", s("createStore")),
            ("name", s("rows")),
            ("options", obj(&[("keyPath", s("id"))])),
        ]),
        obj(&[
            ("op", s("createIndex")),
            ("store", s("rows")),
            ("name", s("byGroup")),
            ("keyPath", s("group")),
        ]),
    ]);
    let open = obj(&[
        ("op", s("openDB")),
        ("db", s("explorer")),
        ("conn", s("explorer")),
        ("version", JsValue::erase(1)),
        ("upgrade", schema),
        ("catch", JsValue::erase(false)),
    ]);
    let puts = array();
    for i in 0..len(rows) {
        push(
            puts,
            obj(&[
                ("op", s("put")),
                ("store", JsValue::null()),
                ("index", JsValue::null()),
                ("args", list(&[at(rows, i)])),
                ("catch", JsValue::erase(false)),
            ]),
        );
    }
    let write = obj(&[
        ("op", s("batch")),
        ("requests", puts),
        ("abort", JsValue::erase(false)),
        ("done", JsValue::erase(true)),
    ]);
    let walk = obj(&[
        ("op", s("openCursor")),
        ("store", JsValue::null()),
        ("index", JsValue::null()),
        ("args", array()),
        ("positions", array()),
        ("rest", list(&[list(&[s("continue")])])),
        ("catch", JsValue::erase(false)),
    ]);
    let steps = list(&[
        open,
        transaction("readwrite", write),
        transaction("readonly", walk),
    ]);
    obj(&[("steps", steps)])
}
/// A transaction step on the Explorer store.
fn transaction(mode: &str, step: JsValue) -> JsValue {
    obj(&[
        ("op", s("tx")),
        ("conn", s("explorer")),
        ("stores", s("rows")),
        ("mode", s(mode)),
        ("steps", list(&[step])),
        ("catch", JsValue::erase(false)),
    ])
}
/// The stored rows' `value` texts, from the cursor entries of the idb log.
fn stored(history: JsValue, decode: bool) -> JsValue {
    let log = get(at(history, 0), "log");
    let values = array();
    for i in 0..len(log) {
        let entry = at(log, i);
        if at(entry, 0).strictly_equals(s("cursor")) {
            let text = get(at(entry, 3), "value");
            push(values, if decode { parse(text) } else { text });
        }
    }
    values
}
fn hits(history: JsValue) -> JsValue {
    get(at(get(at(history, 1), "results"), 1), "hits")
}

/// Returns a component request or the final snapshot. History is private to the caller.
#[js_export]
pub fn matrix_run(packet: JsValue) -> JsValue {
    let input = at(packet, 0);
    let history = at(packet, 1);
    let (name, value) = match len(history) {
        0 => {
            let docs = get(input, "records");
            let rows = array();
            for i in 0..len(docs) {
                let d = at(docs, i);
                push(
                    rows,
                    obj(&[
                        ("id", get(d, "id")),
                        ("group", s("documents")),
                        ("value", stringify(d)),
                    ]),
                );
            }
            (s("idb"), persist(rows))
        }
        1 => {
            let options = obj(&[
                ("fields", list(&[s("title"), s("body")])),
                ("storeFields", list(&[s("body")])),
            ]);
            let add = obj(&[("op", s("addAll")), ("documents", stored(history, true))]);
            let search = obj(&[("op", s("search")), ("query", get(input, "query"))]);
            (
                s("minisearch"),
                obj(&[("options", options), ("operations", list(&[add, search]))]),
            )
        }
        2 => {
            let found = hits(history);
            let markdown = if len(found) == 0 {
                s("")
            } else {
                get(at(found, 0), "body")
            };
            (
                s("marked"),
                obj(&[("op", s("parse")), ("markdown", markdown)]),
            )
        }
        3 => (
            s("fflate"),
            obj(&[
                ("op", s("strToU8")),
                ("text", join(stored(history, false), s("\n"))),
            ]),
        ),
        4 => {
            let data = obj(&[("bytes", at(history, 3))]);
            let options = obj(&[("level", JsValue::erase(6)), ("mtime", JsValue::erase(0))]);
            (
                s("fflate"),
                obj(&[("op", s("gzipSync")), ("data", data), ("options", options)]),
            )
        }
        5 => (s("uplot"), get(input, "chart")),
        _ => {
            let found = hits(history);
            let ids = array();
            for i in 0..len(found) {
                push(ids, get(at(found, i), "id"));
            }
            (
                JsValue::null(),
                list(&[
                    ids,
                    at(history, 2),
                    at(history, 4),
                    JsValue::erase(len(at(history, 3)) as f64),
                    at(history, 5),
                ]),
            )
        }
    };
    list(&[name, value])
}
