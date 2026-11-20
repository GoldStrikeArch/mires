#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
use alloc::string::{String, ToString};
use mires_js::{
    JsValue, await_promise, callback0, callback1, callback2, construct, future_to_promise, invoke,
    js_bindings, js_export, throw_value, try_host,
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("ky port panicked"))
}

js_bindings! {
    #[js(global, get = "globalThis")]
    fn global() -> JsValue;
    #[js(call = "Reflect.get")]
    fn property(target:JsValue,key:JsValue)->JsValue;
    #[js(call = "Reflect.set")]
    fn assign(target:JsValue,key:JsValue,value:JsValue)->bool;
    #[js(new = "Array")]
    fn array()->JsValue;
    #[js(new = "Object")]
    fn object()->JsValue;
    #[js(index)]
    fn at(target:JsValue,i:usize)->JsValue;
    #[js(get = "length")]
    fn len(target:JsValue)->usize;
}

fn s(value: &str) -> JsValue {
    JsValue::erase(value)
}
fn get(target: JsValue, key: &str) -> JsValue {
    property(target, s(key))
}
fn set(target: JsValue, key: &str, value: JsValue) {
    let _ = assign(target, s(key), value);
}
fn call(target: JsValue, method: &str, args: &[JsValue]) -> JsValue {
    invoke(get(target, method), target, args)
}
fn push(target: JsValue, value: JsValue) {
    call(target, "push", &[value]);
}
fn settle(f: JsValue, value: JsValue) {
    invoke(f, JsValue::undefined(), &[value]);
}

fn new(name: &str, args: &[JsValue]) -> JsValue {
    construct(get(global(), name), args)
}
fn num(n: u32) -> JsValue {
    JsValue::erase(n)
}
fn number(v: JsValue) -> u32 {
    v.read::<f64>() as u32
}
fn text(v: JsValue) -> &'static str {
    v.read::<&str>()
}
fn pack(values: &[JsValue]) -> JsValue {
    let a = array();
    for v in values {
        push(a, *v);
    }
    a
}
fn record(trace: JsValue, fields: &[JsValue]) {
    push(trace, pack(fields));
}
fn header(h: JsValue, key: &str) -> JsValue {
    call(h, "get", &[s(key)])
}
fn named_error(name: &str, message: &str) -> JsValue {
    let err = new("Error", &[s(message)]);
    set(err, "name", s(name));
    err
}

fn canonical_method(method: &str) -> &str {
    for known in [
        "GET", "POST", "PUT", "PATCH", "HEAD", "DELETE", "QUERY", "OPTIONS", "TRACE",
    ] {
        if method.eq_ignore_ascii_case(known) {
            return known;
        }
    }
    method
}

fn build_request(c: JsValue) -> JsValue {
    let headers = new("Headers", &[get(c, "headers")]);
    let init = object();
    let method = get(c, "method");
    if !method.is_undefined() && !method.strictly_equals(JsValue::null()) {
        set(init, "method", s(canonical_method(text(method))));
    }
    set(init, "headers", headers);
    let body = get(c, "json");
    if !body.is_undefined() {
        set(
            init,
            "body",
            call(get(global(), "JSON"), "stringify", &[body]),
        );
        if header(headers, "content-type").strictly_equals(JsValue::null()) {
            call(headers, "set", &[s("content-type"), s("application/json")]);
        }
    }
    if !header(headers, "accept").is_truthy() {
        let accept = if text(get(c, "read")) == "json" {
            "application/json"
        } else {
            "text/*"
        };
        call(headers, "set", &[s("accept"), s(accept)]);
    }
    new("Request", &[get(c, "url"), init])
}

fn fixture_fetch(c: JsValue, req: JsValue, user: JsValue, attempt: u32, trace: JsValue) -> JsValue {
    future_to_promise(async move {
        let h = get(req, "headers");
        let body = await_promise(call(req, "text", &[])).await?;
        record(
            trace,
            &[
                s("fetch"),
                get(req, "method"),
                get(req, "url"),
                header(h, "accept"),
                header(h, "content-type"),
                header(h, "x-matrix"),
                body,
            ],
        );
        if text(get(c, "abort")) == "fetch" {
            call(user, "abort", &[named_error("AbortError", "cancelled")]);
        }
        let signal = get(req, "signal");
        if get(signal, "aborted").is_truthy() {
            return Err(get(signal, "reason"));
        }
        let replies = get(c, "replies");
        let reply = at(replies, (attempt as usize).min(len(replies) - 1));
        match text(get(reply, "kind")) {
            "network" => Err(new("TypeError", &[s("Failed to fetch")])),
            "stall" => {
                let executor = callback2(move |_, reject| {
                    let aborted = callback1(move |_| {
                        settle(reject, get(signal, "reason"));
                        JsValue::undefined()
                    });
                    let once = object();
                    set(once, "once", JsValue::erase(true));
                    call(signal, "addEventListener", &[s("abort"), aborted, once]);
                    JsValue::undefined()
                });
                await_promise(new("Promise", &[executor])).await
            }
            _ => {
                let init = object();
                set(init, "headers", get(reply, "headers"));
                set(init, "status", get(reply, "status"));
                Ok(new("Response", &[get(reply, "body"), init]))
            }
        }
    })
}

async fn dispatch(
    c: JsValue,
    base: JsValue,
    user: JsValue,
    count: u32,
    trace: JsValue,
) -> Result<JsValue, JsValue> {
    let controller = new("AbortController", &[]);
    let signal = call(
        get(global(), "AbortSignal"),
        "any",
        &[pack(&[get(user, "signal"), get(controller, "signal")])],
    );
    let init = object();
    set(init, "signal", signal);
    let req = new("Request", &[call(base, "clone", &[]), init]);
    let fetching = fixture_fetch(c, req, user, count, trace);
    let timeout = get(c, "timeout");
    if !timeout.is_number() {
        return await_promise(fetching).await;
    }
    let holder = object();
    let executor = callback2(move |_, reject| {
        let expired = callback0(move || {
            settle(reject, named_error("TimeoutError", "Request timed out"));
            call(controller, "abort", &[]);
            JsValue::undefined()
        });
        set(
            holder,
            "timer",
            call(global(), "setTimeout", &[expired, timeout]),
        );
        JsValue::undefined()
    });
    let timer = new("Promise", &[executor]);
    let result = await_promise(call(
        get(global(), "Promise"),
        "race",
        &[pack(&[fetching, timer])],
    ))
    .await;
    call(global(), "clearTimeout", &[get(holder, "timer")]);
    result
}

fn method_retries(method: &str, choices: JsValue) -> bool {
    if choices.is_undefined() {
        return matches!(
            method,
            "GET" | "PUT" | "HEAD" | "DELETE" | "OPTIONS" | "TRACE" | "QUERY"
        );
    }
    for i in 0..len(choices) {
        if method == canonical_method(text(at(choices, i))) {
            return true;
        }
    }
    false
}
fn status_retries(status: u32, choices: JsValue) -> bool {
    if choices.is_undefined() {
        return matches!(status, 408 | 413 | 429 | 500 | 502 | 503 | 504);
    }
    for i in 0..len(choices) {
        if status == number(at(choices, i)) {
            return true;
        }
    }
    false
}

fn wait(ms: f64) -> JsValue {
    new(
        "Promise",
        &[callback2(move |resolve, _| {
            call(global(), "setTimeout", &[resolve, JsValue::erase(ms)]);
            JsValue::undefined()
        })],
    )
}

struct Failure {
    name: &'static str,
    status: u32,
    data: JsValue,
    after: JsValue,
}
impl Failure {
    fn plain(name: &'static str) -> Self {
        Self {
            name,
            status: 0,
            data: JsValue::null(),
            after: JsValue::null(),
        }
    }
}
fn error_output(trace: JsValue, count: u32, f: &Failure, hook: bool) -> JsValue {
    if hook {
        record(trace, &[s("beforeError"), num(count), s(f.name)]);
    }
    let status = if f.status == 0 {
        JsValue::null()
    } else {
        num(f.status)
    };
    pack(&[trace, pack(&[s("error"), s(f.name), status, f.data])])
}
fn parse_json(body: JsValue) -> Result<JsValue, JsValue> {
    try_host(|| call(get(global(), "JSON"), "parse", &[body]))
}
async fn http_error(resp: JsValue) -> Result<Failure, JsValue> {
    let status = number(get(resp, "status"));
    let h = get(resp, "headers");
    let kind = header(h, "content-type");
    let body = await_promise(call(resp, "text", &[])).await?;
    let mime = if kind.is_truthy() {
        text(kind)
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    } else {
        String::new()
    };
    let is_json = mime.contains('/')
        && ["+json", "-json", ".json", "/json"]
            .iter()
            .any(|suffix| mime.ends_with(suffix));
    let data = if text(body).is_empty() {
        JsValue::null()
    } else if is_json {
        parse_json(body).unwrap_or(JsValue::null())
    } else {
        body
    };
    Ok(Failure {
        name: "HTTPError",
        status,
        data,
        after: header(h, "retry-after"),
    })
}
fn retry_delay(c: JsValue, method: &str, count: u32, f: &Failure, trace: JsValue) -> Option<f64> {
    if count >= number(get(c, "limit")) || !method_retries(method, get(c, "methods")) {
        return None;
    }
    match f.name {
        "HTTPError" => {
            if !status_retries(f.status, get(c, "statusCodes")) {
                return None;
            }
            if matches!(f.status, 413 | 429 | 503) && f.after.is_truthy() {
                let raw = text(f.after).trim();
                if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
                    let after = call(global(), "Number", &[s(raw)]).read::<f64>() * 1000.0;
                    return Some(after.min(get(c, "maxRetryAfter").read::<f64>()));
                }
            } else if f.status == 413 {
                return None;
            }
        }
        "TimeoutError" => {
            if !get(c, "retryOnTimeout").is_truthy() {
                return None;
            }
        }
        "NetworkError" => {}
        _ => return None,
    }
    let mut delay: f64 = 300.0;
    for _ in 0..count {
        delay *= 2.0;
    }
    record(
        trace,
        &[s("backoff"), num(count + 1), JsValue::erase(delay)],
    );
    Some(delay.min(get(c, "backoffLimit").read::<f64>()))
}
async fn request_case(c: JsValue) -> Result<JsValue, JsValue> {
    let trace = array();
    record(trace, &[s("init")]);
    let user = new("AbortController", &[]);
    if text(get(c, "abort")) == "before" {
        call(user, "abort", &[named_error("AbortError", "cancelled")]);
    }
    let base = match try_host(|| build_request(c)) {
        Ok(base) => base,
        Err(error) => {
            return Ok(error_output(
                trace,
                0,
                &Failure::plain(text(get(error, "name"))),
                false,
            ));
        }
    };
    let h = get(base, "headers");
    let method = text(get(base, "method"));
    let mut count = 0;
    if get(get(user, "signal"), "aborted").is_truthy() {
        return Ok(error_output(
            trace,
            count,
            &Failure::plain("AbortError"),
            true,
        ));
    }
    record(trace, &[s("beforeRequest"), num(0)]);
    call(h, "set", &[s("x-matrix"), s("initial")]);
    loop {
        let error = match dispatch(c, base, user, count, trace).await {
            Ok(resp) => {
                record(
                    trace,
                    &[s("afterResponse"), num(count), get(resp, "status")],
                );
                if !get(resp, "ok").is_truthy() && get(c, "throwHttpErrors").is_truthy() {
                    http_error(resp).await?
                } else {
                    let body = await_promise(call(resp, "text", &[])).await?;
                    let value = if text(get(c, "read")) == "json" {
                        match parse_json(body) {
                            Ok(v) => v,
                            Err(_) => {
                                return Ok(error_output(
                                    trace,
                                    count,
                                    &Failure::plain("SyntaxError"),
                                    false,
                                ));
                            }
                        }
                    } else {
                        body
                    };
                    return Ok(pack(&[trace, pack(&[s("ok"), value])]));
                }
            }
            Err(err) => {
                let name = text(get(err, "name"));
                let wrapped =
                    if name == "TypeError" && text(get(err, "message")) == "Failed to fetch" {
                        "NetworkError"
                    } else if name == "TimeoutError" {
                        "TimeoutError"
                    } else if name == "AbortError" {
                        "AbortError"
                    } else {
                        "TypeError"
                    };
                Failure::plain(wrapped)
            }
        };
        if let Some(delay) = retry_delay(c, method, count, &error, trace) {
            await_promise(wait(delay)).await?;
            count += 1;
            record(trace, &[s("beforeRetry"), num(count), s(error.name)]);
            let mut label = String::from("retry-");
            label.push_str(&count.to_string());
            call(h, "set", &[s("x-matrix"), s(&label)]);
            if text(get(c, "abort")) == "retry" {
                call(user, "abort", &[named_error("AbortError", "cancelled")]);
            }
        } else {
            return Ok(error_output(trace, count, &error, true));
        }
    }
}
#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    future_to_promise(async move {
        let rows = get(input, "cases");
        let results = array();
        for i in 0..len(rows) {
            push(results, request_case(at(rows, i)).await?);
        }
        Ok(results)
    })
}
