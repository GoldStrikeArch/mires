#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;
use mires_js::{js_export, throw_value, try_host};
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

#[js_export]
pub fn matrix_run(input: V) -> V {
    let data = field(input, "data");
    let qs = field(input, "queries");
    let out = array();
    for i in 0..len(qs) {
        let q = at(qs, i);
        let result = match try_host(|| query(q, data)) {
            Ok(result) => result,
            Err(error) if stack_exhausted(error) => throw_value(error),
            Err(_) => {
                let o = obj();
                set(o, "error", s("syntax"));
                o
            }
        };
        push(out, result);
    }
    out
}
