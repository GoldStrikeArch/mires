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
fn stored(history: JsValue, decode: bool) -> JsValue {
    let cursor = at(at(history, 0), 3);
    let values = array();
    for i in 0..len(cursor) {
        let text = at(at(cursor, i), 2);
        push(values, if decode { parse(text) } else { text });
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
            let batch = obj(&[("rows", rows)]);
            (
                s("idb"),
                obj(&[
                    ("batches", list(&[batch])),
                    ("keys", array()),
                    ("groups", array()),
                    ("step", JsValue::erase(1)),
                ]),
            )
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
