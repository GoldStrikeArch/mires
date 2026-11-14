//! The `f64` text call shapes the native route takes in a `js-cleanup=on` build, shared word for
//! word by the JavaScript fixture (`301_f64_format_paths_js.rs`) and the Rust oracle
//! (`f64_to_string_oracle.rs`) that `scripts/native-f64-differential.sh` compares them with.
//! No piece holds a tab or a newline: the oracle writes one tab-separated line per value.

use alloc::string::{String, ToString};

const VERSION: &str = "5.2.0";

/// `api::print_debug`'s shape in the idiomatic FingerprintJS port: a default template over a
/// constant `&str`, a host `&str` and the `f64`.
pub fn debug_line(name: &str, value: f64) -> String {
    alloc::format!("version: {} name: {} value: {} end", VERSION, name, value)
}

/// `f64::to_string`.
pub fn text(value: f64) -> String {
    value.to_string()
}

/// `sources_dom::prepare_natural_fonts`'s shape: the reciprocal's `to_string`.
pub fn reciprocal_text(ratio: f64) -> String {
    (1.0 / ratio).to_string()
}
