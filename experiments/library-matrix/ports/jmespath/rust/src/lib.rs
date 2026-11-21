#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;
use mires_js::{js_export, throw_value};
mod eval;
mod functions;
mod host;
mod parser;
use host::*;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(s("jmespath panic"))
}

fn query(q: V, data: V) -> V {
    let n = parser::compile(field(q, "expression"));
    let result = if field(q, "op").strictly_equals(s("compile")) {
        n
    } else {
        n.and_then(|ast| eval::eval(ast, data))
    };
    let out = obj();
    match result {
        Ok(v) => set(out, "value", v),
        Err(category) => set(out, "error", s(category)),
    }
    out
}

/// The reference adapter's `run`. Only jmespath's errors become categories. Anything thrown inside
/// a query, such as stack exhaustion or the panic handler's throw, propagates out of `matrix_run`,
/// as it does out of the reference adapter, and fails the check.
#[js_export]
pub fn matrix_run(input: V) -> V {
    let data = field(input, "data");
    let qs = field(input, "queries");
    let out = array();
    for i in 0..len(qs) {
        push(out, query(at(qs, i), data));
    }
    out
}
