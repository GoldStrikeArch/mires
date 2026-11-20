#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;

mod diff;
mod patch;
mod utf16;
mod words;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
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
js_bindings! {
    #[js(call="Reflect.get")] fn get(o:JsValue,k:JsValue)->JsValue;
    #[js(index_set)] fn put(o:JsValue,k:JsValue,v:JsValue);
    #[js(index_set)] fn set(a:JsValue,i:usize,v:JsValue);
    #[js(index)] fn at(a:JsValue,i:usize)->JsValue;
    #[js(get="length")] fn len(a:JsValue)->usize;
    #[js(new="Array")] fn array(n:usize)->JsValue;
    #[js(new="Object")] fn obj()->JsValue;
    #[js(call="Object.create")] fn dictionary(proto:JsValue)->JsValue;
    #[js(call="Object.keys")] fn keys(o:JsValue)->JsValue;
    #[js(method="sort")] fn sort_keys(v:JsValue)->JsValue;
    #[js(call="Array.isArray")] fn is_array(o:JsValue)->bool;
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
    let names = sort_keys(keys(v));
    let o = dictionary(JsValue::null());
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
    fn values(v: JsValue) -> Vec<String> {
        let mut out = Vec::new();
        for i in 0..len(v) {
            out.push(text(json(at(v, i))))
        }
        out
    }
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
            old_start: nat(h, "oldStart", 1),
            old_lines: nat(h, "oldLines", 0),
            new_start: nat(h, "newStart", 1),
            new_lines: nat(h, "newLines", 0),
            lines: str_array(field(h, "lines")),
        })
    }
    p
}
fn hunks(p: &Patch, reversed: bool) -> JsValue {
    let hunks = array(p.hunks.len());
    for (i, h) in p.hunks.iter().enumerate() {
        let x = obj();
        if reversed {
            number(x, "oldLines", h.old_lines);
            number(x, "oldStart", h.old_start);
            number(x, "newLines", h.new_lines);
            number(x, "newStart", h.new_start)
        } else {
            number(x, "oldStart", h.old_start);
            number(x, "oldLines", h.old_lines);
            number(x, "newStart", h.new_start);
            number(x, "newLines", h.new_lines)
        }
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
        prop(o, "hunks", hunks(p, false));
        for key in &p.metadata_order {
            prop(o, key, value(p, key))
        }
    } else {
        for key in ["oldFileName", "newFileName", "oldHeader", "newHeader"] {
            prop(o, key, value(p, key))
        }
        prop(o, "hunks", hunks(p, false))
    }
    o
}
fn reverse(v: JsValue) -> JsValue {
    let mut p = read_patch(v);
    let copy = p.copy;
    patch::reverse(&mut p);
    let o = obj();
    let names = keys(v);
    for i in 0..len(names) {
        let key = at(names, i);
        let k = key.read::<&str>();
        if !(copy && (k == "isCreate" || k == "isCopy" || k == "isRename")) {
            put(o, key, get(v, key))
        }
    }
    for k in [
        "oldFileName",
        "oldHeader",
        "newFileName",
        "newHeader",
        "oldMode",
        "newMode",
    ] {
        prop(o, k, value(&p, k))
    }
    if !copy {
        prop(o, "isCreate", field(v, "isDelete"))
    }
    prop(
        o,
        "isDelete",
        if copy {
            JsValue::erase(true)
        } else {
            field(v, "isCreate")
        },
    );
    prop(o, "hunks", hunks(&p, true));
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
            let mut p = if patch.is_string() {
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
            match patch::apply(&text(a), &mut p) {
                Some(s) => utf16::write(&s),
                None => JsValue::erase(false),
            }
        }
        _ => fail(),
    }
}
