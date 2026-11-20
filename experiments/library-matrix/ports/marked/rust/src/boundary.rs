//! JS <-> Rust boundary for the marked 18.1.1 matrix contract (CONTRACT.md, "Input" / "Output").

use alloc::string::String;
use mires_js::{js_bindings, throw_value, JsValue};

use crate::lexer::Lexer;
use crate::renderer::{render_blocks, render_inline};
use crate::tokens::Options;

js_bindings! {
    #[js(call = "Reflect.get")]
    fn property(target: JsValue, key: JsValue) -> JsValue;
}

#[inline]
fn get(object: JsValue, key: &str) -> Option<JsValue> {
    let value = property(object, JsValue::erase(key));
    if value.is_undefined() {
        None
    } else {
        Some(value)
    }
}

fn read_options(input: JsValue) -> Options {
    let mut opts = Options::defaults();
    if let Some(obj) = get(input, "options") {
        if let Some(v) = get(obj, "gfm") {
            opts.gfm = v.is_truthy();
        }
        if let Some(v) = get(obj, "breaks") {
            opts.breaks = v.is_truthy();
        }
        if let Some(v) = get(obj, "pedantic") {
            opts.pedantic = v.is_truthy();
        }
    }
    opts
}

pub fn run(input: JsValue) -> JsValue {
    let op_val = property(input, JsValue::erase("op"));
    let op = op_val.read::<&str>();
    let md_val = property(input, JsValue::erase("markdown"));
    let md = md_val.read::<&str>();
    let opts = read_options(input);

    let mut lexer = Lexer::new(opts);
    let mut out = String::with_capacity(md.len() + (md.len() >> 1));

    if op == "parse" {
        let blocks = lexer.lex(md);
        render_blocks(&blocks, &lexer.inline_slots, &mut out);
    } else if op == "parseInline" {
        let inlines = lexer.lex_inline(md);
        render_inline(&inlines, &mut out);
    } else {
        throw_value(JsValue::erase("marked: unknown operation"));
    }

    JsValue::erase(out.as_str())
}
