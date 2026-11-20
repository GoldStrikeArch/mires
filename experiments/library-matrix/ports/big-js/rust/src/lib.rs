#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;
mod decimal;
use alloc::string::{String, ToString};
use decimal::Decimal;
use mires_js::{js_bindings, js_export, throw_value, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("big.js Rust port panicked"))
}

js_bindings! {
    #[js(call = "Reflect.get")]
    fn property(input:JsValue,key:JsValue)->JsValue;
    #[js(get = "length")]
    fn length(v:JsValue)->usize;
    #[js(index)]
    fn element(v:JsValue,i:usize)->JsValue;
    #[js(call = "Number")]
    fn number(v:JsValue)->f64;
    #[js(call = "Math.sqrt")]
    fn machine_root(v:f64)->f64;
    #[js(call = "String")]
    fn string(v:JsValue)->JsValue;
}
fn root_estimate(s: &str) -> f64 {
    machine_root(number(JsValue::erase(s)))
}
fn number_text(n: f64) -> &'static str {
    string(JsValue::erase(n)).read::<&str>()
}
fn field(v: JsValue, key: &str) -> JsValue {
    property(v, JsValue::erase(key))
}
// A number option: undefined and JSON null are absent, as for the adapter's `??`.
fn number_field(v: JsValue, key: &str) -> Option<f64> {
    let raw = field(v, key);
    if raw.is_undefined() || raw.is_null() {
        None
    } else {
        Some(raw.read::<f64>())
    }
}
// The Big constructor's conversion: a number goes through String(), except that -0 becomes "-0".
fn text(v: JsValue) -> &'static str {
    if v.is_number() {
        let n = v.read::<f64>();
        if n == 0.0 && 1.0 / n < 0.0 {
            return "-0";
        }
    }
    string(v).read::<&str>()
}
fn operation(c: JsValue) -> Result<String, &'static str> {
    let op = field(c, "op").read::<&str>();
    let x = Decimal::parse(text(field(c, "x")))?;
    let dp = number_field(c, "dp").unwrap_or(20.0);
    let rm = number_field(c, "rm").unwrap_or(1.0);
    let n = number_field(c, "n");
    let mode = number_field(c, "mode").unwrap_or(rm);
    let result = match op {
        "plus" | "minus" | "times" | "div" | "cmp" => {
            let y = Decimal::parse(text(field(c, "y")))?;
            match op {
                "plus" | "minus" => x.add(&y, op == "minus"),
                "times" => x.times(&y),
                "div" => x.div(&y, dp, rm)?,
                _ => return Ok(x.cmp(&y).to_string()),
            }
        }
        "pow" => x.pow(n, dp, rm)?,
        "sqrt" => x.sqrt(dp, rm)?,
        "round" => x.round(n, mode)?,
        "toFixed" | "toExponential" | "toPrecision" | "toString" => {
            return x.format(op, if op == "toString" { None } else { n }, mode);
        }
        _ => return Err("unknown decimal operation"),
    };
    result.format("toString", None, mode)
}
#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let cases = field(input, "cases");
    let repeats = number_field(input, "repeat").unwrap_or(1.0) as usize;
    let mut result = String::new();
    for k in 0..repeats {
        for i in 0..length(cases) {
            if k != 0 || i != 0 {
                result.push('\n');
            }
            match operation(element(cases, i)) {
                Ok(s) => result.push_str(&s),
                Err(s) => result.push_str(s),
            }
        }
    }
    JsValue::erase(result.as_str())
}
