#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;

mod diff;
mod patch;
mod utf16;
mod words;

use alloc::{string::String, vec::Vec};
use diff::{Options, ADD, CHARS, JSON, LINES, REMOVE, WORDS};
use mires_js::{js_bindings, js_export, throw_value, JsValue};
use patch::{Hunk, Patch};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    fail()
}
fn fail() -> ! {
    throw_value(JsValue::erase("invalid diff operation or patch"))
}
fn type_error(message: &str) -> ! {
    throw_value(JsValue::erase(message))
}
js_bindings! {
    #[js(call="Reflect.get")] fn get(o:JsValue,k:JsValue)->JsValue;
    #[js(index_set)] fn put(o:JsValue,k:JsValue,v:JsValue);
    #[js(index_set)] fn set(a:JsValue,i:usize,v:JsValue);
    #[js(index)] fn at(a:JsValue,i:usize)->JsValue;
    #[js(get="length")] fn len(a:JsValue)->usize;
    #[js(new="Array")] fn array(n:usize)->JsValue;
    #[js(new="Object")] fn obj()->JsValue;
    #[js(call="Reflect.deleteProperty")] fn delete(o:JsValue,k:JsValue)->bool;
    #[js(call="Object.keys")] fn keys(o:JsValue)->JsValue;
    #[js(method="sort")] fn sort_keys(v:JsValue)->JsValue;
    #[js(call="Array.isArray")] fn is_array(o:JsValue)->bool;
    #[js(method="toLowerCase")] fn to_lower_case(v:JsValue)->JsValue;
    #[js(call="JSON.stringify")] fn json(v:JsValue)->JsValue;
    #[js(call="JSON.stringify")] fn pretty(v:JsValue,replacer:JsValue,space:u32)->JsValue;
}
fn field(o: JsValue, k: &str) -> JsValue {
    if o.is_undefined() || o.is_null() {
        return JsValue::undefined();
    }
    get(o, JsValue::erase(k))
}
fn prop(o: JsValue, k: &str, v: JsValue) {
    put(o, JsValue::erase(k), v)
}
fn number(o: JsValue, k: &str, n: usize) {
    prop(o, k, JsValue::erase(n as u32))
}
fn float(o: JsValue, k: &str) -> f64 {
    let x = field(o, k);
    if x.is_number() {
        x.read::<f64>()
    } else {
        f64::NAN
    }
}
fn text(v: JsValue) -> String {
    utf16::read(v)
}
fn optional(o: JsValue, k: &str) -> Option<String> {
    let v = field(o, k);
    if v.is_string() {
        Some(text(v))
    } else {
        None
    }
}
fn flag(o: JsValue, k: &str) -> bool {
    field(o, k).is_truthy()
}
fn nat(o: JsValue, k: &str, d: usize) -> usize {
    let x = field(o, k);
    if x.is_number() {
        x.read::<f64>() as usize
    } else {
        d
    }
}
fn str_array(v: JsValue) -> Vec<String> {
    let mut result = Vec::new();
    for i in 0..len(v) {
        result.push(text(at(v, i)))
    }
    result
}
fn canonical(v: JsValue) -> JsValue {
    if is_array(v) {
        let a = array(len(v));
        for i in 0..len(v) {
            set(a, i, canonical(at(v, i)))
        }
        return a;
    }
    if !v.is_object() {
        return v;
    }
    // canonicalize() calls a truthy toJSON member, which JSON cannot make callable.
    if field(v, "toJSON").is_truthy() {
        type_error("obj.toJSON is not a function")
    }
    // canonicalize() assigns the sorted keys to a plain object, so a
    // "__proto__" key sets its prototype and is not serialized.
    let names = sort_keys(keys(v));
    let o = obj();
    for i in 0..len(names) {
        let key = at(names, i);
        put(o, key, canonical(get(v, key)))
    }
    o
}
fn doc(v: JsValue, mode: u8) -> String {
    if mode == JSON && !v.is_string() {
        text(pretty(canonical(v), JsValue::null(), 2))
    } else {
        text(v)
    }
}
fn write_change(kind: u8, count: usize, value: JsValue) -> JsValue {
    let o = obj();
    number(o, "count", count);
    prop(o, "added", JsValue::erase(kind == ADD));
    prop(o, "removed", JsValue::erase(kind == REMOVE));
    prop(o, "value", value);
    o
}
fn array_diff(a: JsValue, b: JsValue, o: &Options) -> JsValue {
    // Elements compare by their JSON text, which is `===` for JSON scalars. With ignoreCase,
    // elements are strings (CONTRACT.md) and compare by JS `toLowerCase()`.
    let values = |v: JsValue| {
        let mut out = Vec::new();
        for i in 0..len(v) {
            let item = at(v, i);
            out.push(if o.ignore_case {
                text(to_lower_case(item))
            } else {
                text(json(item))
            })
        }
        out
    };
    let changes = diff::myers(&values(a), &values(b), o.separate);
    let out = array(changes.len());
    let mut old = 0;
    let mut new = 0;
    for (i, c) in changes.iter().enumerate() {
        let value = array(c.count);
        for j in 0..c.count {
            set(
                value,
                j,
                if c.kind == REMOVE {
                    at(a, old + j)
                } else {
                    at(b, new + j)
                },
            )
        }
        set(out, i, write_change(c.kind, c.count, value));
        if c.kind != ADD {
            old += c.count
        }
        if c.kind != REMOVE {
            new += c.count
        }
    }
    out
}
fn meta(o: JsValue) -> Patch {
    Patch {
        old_name: optional(o, "oldFileName"),
        new_name: optional(o, "newFileName"),
        old_header: optional(o, "oldHeader"),
        new_header: optional(o, "newHeader"),
        old_mode: optional(o, "oldMode"),
        new_mode: optional(o, "newMode"),
        git: flag(o, "isGit"),
        index: optional(o, "index"),
        create: flag(o, "isCreate"),
        delete: flag(o, "isDelete"),
        copy: flag(o, "isCopy"),
        rename: flag(o, "isRename"),
        binary: flag(o, "isBinary"),
        hunks: Vec::new(),
        metadata_order: Vec::new(),
    }
}
fn read_patch(o: JsValue) -> Patch {
    let mut p = meta(o);
    let hunks = field(o, "hunks");
    for i in 0..len(hunks) {
        let h = at(hunks, i);
        p.hunks.push(Hunk {
            old_start: float(h, "oldStart"),
            old_lines: float(h, "oldLines"),
            new_start: float(h, "newStart"),
            new_lines: float(h, "newLines"),
            lines: str_array(field(h, "lines")),
        })
    }
    p
}
fn hunks(p: &Patch) -> JsValue {
    let hunks = array(p.hunks.len());
    for (i, h) in p.hunks.iter().enumerate() {
        let x = obj();
        prop(x, "oldStart", JsValue::erase(h.old_start));
        prop(x, "oldLines", JsValue::erase(h.old_lines));
        prop(x, "newStart", JsValue::erase(h.new_start));
        prop(x, "newLines", JsValue::erase(h.new_lines));
        let lines = array(h.lines.len());
        for (j, l) in h.lines.iter().enumerate() {
            set(lines, j, utf16::write(&l))
        }
        prop(x, "lines", lines);
        set(hunks, i, x)
    }
    hunks
}
fn named(v: &Option<String>) -> JsValue {
    match v {
        Some(s) => utf16::write(&s),
        None => JsValue::undefined(),
    }
}
fn value(p: &Patch, key: &str) -> JsValue {
    match key {
        "oldFileName" => named(&p.old_name),
        "newFileName" => named(&p.new_name),
        "oldHeader" => named(&p.old_header),
        "newHeader" => named(&p.new_header),
        "oldMode" => named(&p.old_mode),
        "newMode" => named(&p.new_mode),
        "index" => named(&p.index),
        _ => JsValue::erase(true),
    }
}
fn write_patch(p: &Patch, parsed: bool) -> JsValue {
    let o = obj();
    if parsed {
        prop(o, "hunks", hunks(p));
        for key in &p.metadata_order {
            prop(o, key, value(p, key))
        }
    } else {
        for key in ["oldFileName", "newFileName", "oldHeader", "newHeader"] {
            prop(o, key, value(p, key))
        }
        prop(o, "hunks", hunks(p))
    }
    o
}
/// `swapPrefix`, applied to Git names.
fn swap_prefix(v: JsValue) -> JsValue {
    if !v.is_string() {
        return v;
    }
    let name = text(v);
    if let Some(rest) = name.strip_prefix("a/") {
        utf16::write(&(String::from("b/") + rest))
    } else if let Some(rest) = name.strip_prefix("b/") {
        utf16::write(&(String::from("a/") + rest))
    } else {
        v
    }
}
fn reverse_line(l: JsValue) -> JsValue {
    let s = text(l);
    if let Some(rest) = s.strip_prefix('-') {
        utf16::write(&(String::from("+") + rest))
    } else if let Some(rest) = s.strip_prefix('+') {
        utf16::write(&(String::from("-") + rest))
    } else {
        l
    }
}
fn reverse_hunk(h: JsValue) -> JsValue {
    let x = obj();
    prop(x, "oldLines", field(h, "newLines"));
    prop(x, "oldStart", field(h, "newStart"));
    prop(x, "newLines", field(h, "oldLines"));
    prop(x, "newStart", field(h, "oldStart"));
    let lines = field(h, "lines");
    let reversed = array(len(lines));
    for i in 0..len(lines) {
        set(reversed, i, reverse_line(at(lines, i)))
    }
    prop(x, "lines", reversed);
    x
}
/// `reversePatch` on one structured patch: a copy of its properties with the
/// sides swapped, as `Object.assign({}, patch, {...})` builds it.
fn reverse(v: JsValue) -> JsValue {
    let git = flag(v, "isGit");
    let name = |k: &str| {
        if git {
            swap_prefix(field(v, k))
        } else {
            field(v, k)
        }
    };
    let o = obj();
    let names = keys(v);
    for i in 0..len(names) {
        let key = at(names, i);
        put(o, key, get(v, key))
    }
    prop(o, "oldFileName", name("newFileName"));
    prop(o, "oldHeader", field(v, "newHeader"));
    prop(o, "newFileName", name("oldFileName"));
    prop(o, "newHeader", field(v, "oldHeader"));
    prop(o, "oldMode", field(v, "newMode"));
    prop(o, "newMode", field(v, "oldMode"));
    prop(o, "isCreate", field(v, "isDelete"));
    prop(o, "isDelete", field(v, "isCreate"));
    let source = field(v, "hunks");
    let reversed = array(len(source));
    for i in 0..len(source) {
        set(reversed, i, reverse_hunk(at(source, i)))
    }
    prop(o, "hunks", reversed);
    if flag(v, "isCopy") {
        // Reversing a copy deletes the copy destination.
        prop(o, "newFileName", JsValue::erase("/dev/null"));
        prop(o, "newHeader", JsValue::undefined());
        prop(o, "isDelete", JsValue::erase(true));
        for k in ["isCreate", "isCopy", "isRename"] {
            delete(o, JsValue::erase(k));
        }
        prop(o, "hunks", array(0))
    }
    o
}
#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let op = text(field(input, "op"));
    let options = field(input, "options");
    let a = field(input, "before");
    let b = field(input, "after");
    let patch = field(input, "patch");
    let o = Options {
        ignore_case: flag(options, "ignoreCase"),
        separate: flag(options, "oneChangePerToken"),
        strip_cr: flag(options, "stripTrailingCr"),
        ignore_ws: flag(options, "ignoreWhitespace"),
        newline_token: flag(options, "newlineIsToken"),
        ignore_eof: flag(options, "ignoreNewlineAtEof"),
    };
    match op.as_str() {
        "diffArrays" => array_diff(a, b, &o),
        "diffChars" | "diffWords" | "diffLines" | "diffJson" => {
            let mode = match op.as_str() {
                "diffWords" => WORDS,
                "diffLines" => LINES,
                "diffJson" => JSON,
                _ => CHARS,
            };
            let changes = diff::changes(
                mode,
                &diff::tokens(mode, &doc(a, mode), &o),
                &diff::tokens(mode, &doc(b, mode), &o),
                &o,
            );
            let out = array(changes.len());
            for (i, c) in changes.iter().enumerate() {
                set(
                    out,
                    i,
                    write_change(c.kind, c.count, utf16::write(&c.value)),
                )
            }
            out
        }
        "structuredPatch" | "createTwoFilesPatch" => {
            let p = patch::create(&text(a), &text(b), nat(options, "context", 4), meta(input));
            if op == "structuredPatch" {
                write_patch(&p, false)
            } else {
                utf16::write(&patch::format(&p))
            }
        }
        "parsePatch" => {
            let ps = match patch::parse(&text(patch)) {
                Ok(v) => v,
                Err(_) => fail(),
            };
            let out = array(ps.len());
            for (i, p) in ps.iter().enumerate() {
                set(out, i, write_patch(p, true))
            }
            out
        }
        "reversePatch" => {
            if is_array(patch) {
                let n = len(patch);
                let out = array(n);
                for i in 0..n {
                    set(out, n - 1 - i, reverse(at(patch, i)))
                }
                out
            } else {
                reverse(patch)
            }
        }
        "applyPatch" => {
            let p = if patch.is_string() {
                let mut ps = match patch::parse(&text(patch)) {
                    Ok(v) => v,
                    Err(_) => fail(),
                };
                if ps.len() != 1 {
                    fail()
                }
                ps.pop().unwrap()
            } else if is_array(patch) {
                if len(patch) != 1 {
                    fail()
                }
                read_patch(at(patch, 0))
            } else {
                read_patch(patch)
            };
            match patch::apply(&text(a), p) {
                Some(s) => utf16::write(&s),
                None => JsValue::erase(false),
            }
        }
        _ => fail(),
    }
}
