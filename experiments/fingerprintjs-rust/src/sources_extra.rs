//! Synchronous sources whose exact behavior is best expressed at the host boundary.

use mires_js::{invoke, throw_value, try_host, JsValue};

use crate::{document, get, object, set, window};

fn math_call(name: &str, value: f64) -> f64 {
    let math = get(window(), "Math");
    let function = get(math, name);
    if function.is_function() {
        invoke(function, math, &[JsValue::erase(value)]).read()
    } else {
        0.0
    }
}

fn math_pow(base: f64, exponent: f64) -> f64 {
    let math = get(window(), "Math");
    invoke(
        get(math, "pow"),
        math,
        &[JsValue::erase(base), JsValue::erase(exponent)],
    )
    .read()
}

fn math_native(name: &str, value: f64) -> JsValue {
    JsValue::erase(math_call(name, value))
}

/// The 24-value floating-point fingerprint in its observable insertion order.
#[must_use]
#[allow(clippy::manual_midpoint)]
pub fn source_math(_: JsValue) -> JsValue {
    let output = object();
    let log = |value| math_call("log", value);
    let sqrt = |value| math_call("sqrt", value);
    let exp = |value| math_call("exp", value);

    set(
        output,
        "acos",
        math_native("acos", 0.123_124_234_234_234_24),
    );
    set(output, "acosh", math_native("acosh", 1e308));
    set(
        output,
        "acoshPf",
        JsValue::erase(log(1e154 + sqrt(1e154 * 1e154 - 1.0))),
    );
    set(
        output,
        "asin",
        math_native("asin", 0.123_124_234_234_234_24),
    );
    set(output, "asinh", math_native("asinh", 1.0));
    set(output, "asinhPf", JsValue::erase(log(1.0 + sqrt(2.0))));
    set(output, "atanh", math_native("atanh", 0.5));
    set(
        output,
        "atanhPf",
        JsValue::erase(log((1.0 + 0.5) / (1.0 - 0.5)) / 2.0),
    );
    set(output, "atan", math_native("atan", 0.5));
    set(output, "sin", math_native("sin", -1e300));
    set(output, "sinh", math_native("sinh", 1.0));
    set(
        output,
        "sinhPf",
        JsValue::erase(exp(1.0) - 1.0 / exp(1.0) / 2.0),
    );
    set(output, "cos", math_native("cos", 10.000_000_000_123));
    set(output, "cosh", math_native("cosh", 1.0));
    set(
        output,
        "coshPf",
        JsValue::erase((exp(1.0) + 1.0 / exp(1.0)) / 2.0),
    );
    set(output, "tan", math_native("tan", -1e300));
    set(output, "tanh", math_native("tanh", 1.0));
    set(
        output,
        "tanhPf",
        JsValue::erase((exp(2.0) - 1.0) / (exp(2.0) + 1.0)),
    );
    set(output, "exp", math_native("exp", 1.0));
    set(output, "expm1", math_native("expm1", 1.0));
    set(output, "expm1Pf", JsValue::erase(exp(1.0) - 1.0));
    set(output, "log1p", math_native("log1p", 10.0));
    set(output, "log1pPf", JsValue::erase(log(11.0)));
    let pi: f64 = get(get(window(), "Math"), "PI").read();
    set(output, "powPI", JsValue::erase(math_pow(pi, -100.0)));
    output
}

/// Whether an ancestor window has another origin.
pub fn is_any_parent_cross_origin() -> Result<bool, JsValue> {
    let mut current = window();
    loop {
        let parent = get(current, "parent");
        if !parent.is_truthy() || parent.strictly_equals(current) {
            return Ok(false);
        }
        let origins = try_host(move || {
            let parent_origin = get(get(parent, "location"), "origin");
            let current_origin = get(get(current, "location"), "origin");
            JsValue::erase(parent_origin.strictly_equals(current_origin))
        });
        match origins {
            Ok(equal) if !equal.read::<bool>() => return Ok(true),
            Ok(_) => current = parent,
            Err(error) => {
                let security = try_host(move || {
                    let name = get(error, "name");
                    JsValue::erase(name.is_string() && name.read::<&str>() == "SecurityError")
                })?;
                if security.read() {
                    return Ok(true);
                }
                return Err(error);
            }
        }
    }
}

fn contains_words_in_order(value: &str, first: &str, second: &str) -> bool {
    fn boundary(value: &[u8], index: usize) -> bool {
        value
            .get(index)
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
    }

    let lower = value.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut search = 0;
    while let Some(relative) = lower[search..].find(first) {
        let start = search + relative;
        let end = start + first.len();
        if boundary(bytes, start.wrapping_sub(1)) && boundary(bytes, end) {
            let mut second_search = end;
            while let Some(relative_second) = lower[second_search..].find(second) {
                let second_start = second_search + relative_second;
                let second_end = second_start + second.len();
                if boundary(bytes, second_start.wrapping_sub(1)) && boundary(bytes, second_end) {
                    return true;
                }
                second_search = second_end;
            }
        }
        search = end;
    }
    false
}

/// Apple Pay availability with exact frame and insecure-context sentinels.
#[must_use]
pub fn source_apple_pay(_: JsValue) -> JsValue {
    let session = get(window(), "ApplePaySession");
    if session.is_undefined() || session.is_null() {
        return JsValue::erase(-1_i32);
    }
    let can_make_payments = get(session, "canMakePayments");
    if !can_make_payments.is_function() {
        return JsValue::erase(-1_i32);
    }

    match is_any_parent_cross_origin() {
        Ok(true) => return JsValue::erase(-3_i32),
        Ok(false) => {}
        Err(error) => throw_value(error),
    }

    match try_host(move || invoke(can_make_payments, session, &[])) {
        Ok(result) => JsValue::erase(result.is_truthy()),
        Err(error) => {
            let classified = try_host(move || {
                let name = get(error, "name");
                let message = get(error, "message");
                JsValue::erase(
                    name.is_string()
                        && name.read::<&str>() == "InvalidAccessError"
                        && message.is_string()
                        && contains_words_in_order(message.read(), "from", "insecure"),
                )
            });
            match classified {
                Ok(value) if value.read::<bool>() => JsValue::erase(-2_i32),
                Ok(_) | Err(_) => throw_value(error),
            }
        }
    }
}

/// The document value used by the Apple Pay frame detector.
#[must_use]
pub fn apple_pay_document() -> JsValue {
    document()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_apple_pay_insecure_messages() {
        assert!(contains_words_in_order(
            "Must not call this from an insecure document",
            "from",
            "insecure"
        ));
        assert!(!contains_words_in_order(
            "transformed insecurely",
            "from",
            "insecure"
        ));
    }
}
