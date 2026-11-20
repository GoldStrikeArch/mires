//! A dispatch loop whose last case is the function's normal return (MIRES-017).
//!
//! `parse_block` is the MIRES-017 repro: a loop with owned locals (`String`, `Vec<String>`,
//! `Vec<Node>`), an early `?`, several `break`s and a `continue`. With its cleanup edges the loop
//! has more than one entry, and the structurizer expresses it as a nested dispatch,
//! `for (;;) { switch ($s0) { case ...: } break; }`, whose cases fall through into each other.
//! The backend prints a `default: throw new Error("rustc_codegen_js: bad block " + $s0)` guard
//! after the last case. Under `JSC_RUST_OPT_LEVEL=z` that last case is the epilogue building
//! `Some(Node::Block { .. })`, and it fell off its end into the guard, so every non-empty call
//! threw `bad block`. It now ends in `break`, onto the `break` after the switch.
//!
//! At the default opt-level the dispatch's last case is a cleanup path that rethrows, so the fixture
//! only failed in the gate's inline-on mode; `351_from_utf8_lossy` reaches the same bug through
//! `core` in every mode.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::String;
use alloc::vec::Vec;

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

/// The repro of MIRES-017, verbatim from `mires-bugs/MIRES-017-switch-last-case-missing-break`.
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

fn show(node: Option<Node>) {
    match node {
        None => print_str("none"),
        Some(Node::Block {
            raw,
            text,
            children,
        }) => {
            print_str(&raw);
            print_usize(text.len());
            print_usize(children.len());
            for child in &children {
                match child {
                    Node::Leaf(s) => print_str(s),
                    Node::Block { raw, .. } | Node::List { raw } => print_str(raw),
                }
            }
        }
        Some(Node::Leaf(s)) | Some(Node::List { raw: s }) => print_str(&s),
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    show(parse_block("a"));
    show(parse_block("> a\n> b\nc"));
    show(parse_block("x\n> y\n> z\nw\n> v"));
    show(parse_block(""));
}
