//! MiniSearch 7.2.0 in Rust, compiled to JavaScript by mires: a faithful port of the subset
//! `libraries/minisearch/CONTRACT.md` defines.
//!
//! - `searchable_map`: the radix tree (`SearchableMap`), with its prefix and fuzzy walks;
//! - `index`: the `MiniSearch` class (documents, field lengths, BM25+, search, autoSuggest,
//!   discarding and vacuuming);
//! - `options`: search options and their spread-merging;
//! - `text` and `separators`: UTF-16 strings, the default tokenizer and `toLowerCase`;
//! - `boundary`: the corpus input into Rust values and the result back to a host object.
//!
//! `matrix_run` is re-exported as `run` by the builder's facade.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod boundary;
mod index;
mod options;
mod searchable_map;
mod separators;
mod text;

use mires_js::{js_bindings, js_export, throw_value, JsValue};

js_bindings! {
    /// `Math.log`: the BM25 formula's logarithm, the same function MiniSearch calls.
    #[js(call = "Math.log")]
    pub(crate) fn math_log(value: f64) -> f64;
}

/// A panic is a bug in the port (or an input outside the parity domain); it surfaces as a host
/// exception instead of hanging the caller.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("minisearch (Rust port): panic"))
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    boundary::run(input)
}
