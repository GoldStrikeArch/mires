//! Ky 2.2.0 in Rust, compiled to JavaScript by mires: a port of the subset that
//! `libraries/ky/CONTRACT.md` defines, and of what `libraries/ky/reference.mjs` does with it.
//!
//! - `ky.rs`: Ky's request, retry loop, option checks, error data and errors;
//! - `retry_timing.rs`: `core/retry-timing.js`, the server-provided retry delays;
//! - `host.rs`: the Fetch, abort, decoding, timer and JSON APIs Ky drives;
//! - this file: the reference adapter (each case's transport, hooks and trace) and the export.
//!
//! The boundary: each corpus case is read in place, field by field. The option values Ky
//! validates stay the JS values they are until Ky's own checks read them, and each result is
//! the JS array `[trace, outcome]`.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod host;
mod ky;
mod retry_timing;

use alloc::rc::Rc;
use alloc::string::{String, ToString};
use core::cell::Cell;
use host::{AbortController, Request, Response, field, list, math_pow, present, s, text};
use ky::{Callbacks, Failure, Options, RetryOptions, Shortcut};
use mires_js::{
    JsValue, await_promise, callback0, callback2, future_to_promise, invoke, js_export, throw_value,
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("ky port panicked"))
}

/// What the adapter aborts its controller with.
fn cancel(controller: AbortController) {
    let reason = host::new_error("cancelled");
    host::set_field(reason, "name", s("AbortError"));
    controller.abort_with(reason);
}

fn nullable(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::null(), s)
}

/// One request case of the reference adapter: its replies and abort point, the trace its hooks
/// record, its abort controller, and how many replies the transport has given.
struct Fixture {
    case: JsValue,
    trace: JsValue,
    controller: AbortController,
    sent: Rc<Cell<usize>>,
}

impl Fixture {
    fn record(&self, entry: &[JsValue]) {
        host::push(self.trace, list(entry));
    }
}

impl Callbacks for Fixture {
    /// The injected transport: it records what it was sent and answers with the next reply.
    fn fetch(&self, request: Request) -> JsValue {
        let (case, trace, controller, sent) = (self.case, self.trace, self.controller, self.sent.clone());
        future_to_promise(async move {
            let body = request.text().await?;
            let headers = request.headers();
            host::push(
                trace,
                list(&[
                    s("fetch"),
                    s(request.method()),
                    s(request.url()),
                    nullable(headers.get("accept")),
                    nullable(headers.get("content-type")),
                    nullable(headers.get("x-matrix")),
                    s(body),
                ]),
            );
            if text(field(case, "abort")) == "fetch" {
                cancel(controller);
            }
            let signal = request.signal();
            signal.throw_if_aborted()?;
            let replies = field(case, "replies");
            let reply = host::at(replies, sent.get().min(host::length(replies) - 1));
            sent.set(sent.get() + 1);
            match text(field(reply, "kind")) {
                "network" => Err(host::new_type_error("Failed to fetch")),
                "stall" => {
                    let stalled = host::promise(callback2(move |_, reject| {
                        signal.on_abort(callback0(move || {
                            invoke(reject, JsValue::undefined(), &[signal.reason()])
                        }));
                        JsValue::undefined()
                    }));
                    await_promise(stalled).await
                }
                _ => Response::new(field(reply, "body"), field(reply, "status"), field(reply, "headers"))
                    .map(Response::as_value),
            }
        })
    }

    fn delay(&self, attempt_count: u32) -> f64 {
        let milliseconds = 300.0 * math_pow(2.0, f64::from(attempt_count) - 1.0);
        self.record(&[s("backoff"), JsValue::erase(attempt_count), JsValue::erase(milliseconds)]);
        milliseconds
    }

    fn init(&self) {
        self.record(&[s("init")]);
    }

    fn before_request(&self, request: Request, retry_count: u32) {
        self.record(&[s("beforeRequest"), JsValue::erase(retry_count)]);
        // A valid name and value: `set` cannot fail.
        let _ = request.headers().set("x-matrix", s("initial"));
    }

    fn after_response(&self, _: Request, response: Response, retry_count: u32) {
        self.record(&[
            s("afterResponse"),
            JsValue::erase(retry_count),
            JsValue::erase(response.status()),
        ]);
    }

    fn before_retry(&self, request: Request, error: &Failure, retry_count: u32) {
        self.record(&[s("beforeRetry"), JsValue::erase(retry_count), s(error.name())]);
        let mut label = String::from("retry-");
        label.push_str(&retry_count.to_string());
        let _ = request.headers().set("x-matrix", s(&label));
        if text(field(self.case, "abort")) == "retry" {
            cancel(self.controller);
        }
    }

    fn before_error(&self, _: Request, error: &Failure, retry_count: u32) {
        self.record(&[s("beforeError"), JsValue::erase(retry_count), s(error.name())]);
    }
}

async fn request_case(case: JsValue) -> JsValue {
    let trace = host::array();
    let controller = AbortController::new();
    if text(field(case, "abort")) == "before" {
        cancel(controller);
    }
    let options = Options {
        method: present(case, "method").map(text),
        headers: present(case, "headers"),
        json: present(case, "json"),
        retry: RetryOptions {
            limit: present(case, "limit"),
            methods: present(case, "methods"),
            status_codes: present(case, "statusCodes"),
            max_retry_after: present(case, "maxRetryAfter"),
            backoff_limit: present(case, "backoffLimit"),
            retry_on_timeout: present(case, "retryOnTimeout"),
        },
        timeout: present(case, "timeout"),
        throw_http_errors: present(case, "throwHttpErrors"),
        signal: Some(controller.signal()),
        callbacks: Fixture { case, trace, controller, sent: Rc::new(Cell::new(0)) },
    };
    let shortcut = if text(field(case, "read")) == "json" { Shortcut::Json } else { Shortcut::Text };
    let outcome = match ky::request(text(field(case, "url")), options, shortcut).await {
        Ok(result) => list(&[s("ok"), result]),
        Err(Failure::Http { response, data, .. }) => list(&[
            s("error"),
            s("HTTPError"),
            JsValue::erase(response.status()),
            data.unwrap_or(JsValue::null()),
        ]),
        Err(error) => list(&[s("error"), s(error.name()), JsValue::null(), JsValue::null()]),
    };
    list(&[trace, outcome])
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    future_to_promise(async move {
        let cases = field(input, "cases");
        let results = host::array();
        for index in 0..host::length(cases) {
            host::push(results, request_case(host::at(cases, index)).await);
        }
        Ok(results)
    })
}
