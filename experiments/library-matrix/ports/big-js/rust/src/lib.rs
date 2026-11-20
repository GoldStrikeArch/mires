#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;
mod decimal;
use alloc::string::{String, ToString};
use decimal::{Decimal, POWER};
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
    #[js(call = "Math.floor")]
    fn floor(v:f64)->f64;
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
fn integer(v: JsValue, key: &str) -> Option<i32> {
    let raw = field(v, key);
    if raw.is_undefined() {
        return None;
    }
    let value = raw.read::<f64>();
    let n = value as i32;
    Some(if n as f64 == value { n } else { i32::MIN })
}
fn operation(c: JsValue) -> Result<String, &'static str> {
    let op = field(c, "op").read::<&str>();
    let x = Decimal::parse(string(field(c, "x")).read::<&str>())?;
    let dp = integer(c, "dp").unwrap_or(20);
    let rm = integer(c, "rm").unwrap_or(1);
    let n = integer(c, "n");
    let mode = integer(c, "mode").unwrap_or(rm);
    let result = match op {
        "plus" | "minus" | "times" | "div" | "cmp" => {
            let y = Decimal::parse(string(field(c, "y")).read::<&str>())?;
            match op {
                "plus" | "minus" => x.add(&y, op == "minus"),
                "times" => x.times(&y),
                "div" => x.div(&y, dp, rm)?,
                _ => return Ok(x.cmp(&y).to_string()),
            }
        }
        "pow" => x.pow(n.ok_or(POWER)?, dp, rm)?,
        "sqrt" => x.sqrt(dp, rm)?,
        "round" => x.round(n.unwrap_or(0), mode)?,
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
    let repeats = integer(input, "repeat").unwrap_or(1);
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
