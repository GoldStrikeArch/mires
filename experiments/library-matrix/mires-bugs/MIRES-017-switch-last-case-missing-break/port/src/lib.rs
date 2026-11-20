//! mires bug repro (MIRES-017): structurizer switch dispatcher omits `break;` on its final
//! case before `default: throw new Error("rustc_codegen_js: bad block " + d)`.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use mires_js::{js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("panic"))
}

enum Node {
    Leaf(String),
    Block {
        raw: String,
        text: String,
        children: Vec<Node>,
    },
    List {
        raw: String,
    },
}

#[inline(never)]
fn match_prefix(s: &str) -> Option<usize> {
    if s.is_empty() { None } else { Some(s.len()) }
}

#[inline(never)]
fn parse_block(src: &str) -> Option<Node> {
    let cap_len = match_prefix(src)?;
    let mut lines: Vec<String> = src[..cap_len].split('\n').map(String::from).collect();
    let mut raw = String::new();
    let mut text = String::new();
    let mut children: Vec<Node> = Vec::new();

    while !lines.is_empty() {
        let mut seen = false;
        let mut split_at = 0usize;
        while split_at < lines.len() {
            if lines[split_at].starts_with('>') {
                seen = true;
                split_at += 1;
            } else if !seen {
                split_at += 1;
            } else {
                break;
            }
        }
        let remaining = lines.split_off(split_at);
        let current_raw = lines.join("\n");
        let current_text = current_raw.clone();

        if raw.is_empty() {
            raw.push_str(&current_raw);
        } else {
            raw.push('\n');
            raw.push_str(&current_raw);
        }
        if text.is_empty() {
            text.push_str(&current_text);
        } else {
            text.push('\n');
            text.push_str(&current_text);
        }
        children.push(Node::Leaf(current_text));

        lines = remaining;
        if lines.is_empty() {
            break;
        }

        let last_kind = match children.last() {
            Some(Node::Leaf(_)) => 1,
            Some(Node::Block { .. }) => 2,
            Some(Node::List { .. }) => 3,
            None => 0,
        };

        if last_kind == 1 {
            break;
        } else if last_kind == 2 {
            let old_raw = match children.last() {
                Some(Node::Block { raw: r, .. }) => r.clone(),
                _ => break,
            };
            let mut combined = String::from(old_raw.as_str());
            combined.push('\n');
            combined.push_str(&lines.join("\n"));
            if let Some(new_node) = parse_block(&combined) {
                if let Some(slot) = children.last_mut() {
                    *slot = new_node;
                }
            }
            break;
        } else if last_kind == 3 {
            let old_raw = match children.last() {
                Some(Node::List { raw: r }) => r.clone(),
                _ => break,
            };
            let mut combined = String::from(old_raw.as_str());
            combined.push('\n');
            combined.push_str(&lines.join("\n"));
            lines = combined.split('\n').map(String::from).collect();
            continue;
        }
    }

    Some(Node::Block {
        raw,
        text,
        children,
    })
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let text: &str = reflect_get(input, JsValue::erase("text")).read();
    match parse_block(text) {
        Some(Node::Block { raw, .. }) => JsValue::erase(raw.as_str()),
        _ => JsValue::erase(text),
    }
}
