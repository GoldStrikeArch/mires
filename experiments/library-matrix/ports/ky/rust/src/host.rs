//! The host APIs Ky drives: Fetch (`Headers`, `Request`, `Response`), `AbortController` and
//! `AbortSignal`, `TextDecoder`, timers, `JSON` and the clock. Ky is a thin layer over them, so
//! the port binds them as they are and every one of their semantics (method and URL
//! normalisation, header validation, body encoding, encoding labels) is the engine's own. Each
//! facade is a `Copy` handle on the host object; a call that can throw returns the thrown value as
//! `Err`.

use mires_js::{JsValue, await_promise, callback2, js_bindings, try_host};

js_bindings! {
    #[js(call = "Reflect.get")] pub fn property(target: JsValue, key: JsValue) -> JsValue;
    #[js(call = "Reflect.set")] pub fn assign(target: JsValue, key: JsValue, value: JsValue) -> bool;
    #[js(call = "Reflect.has")] pub fn has_property(target: JsValue, key: JsValue) -> bool;
    #[js(call = "Reflect.apply")] fn reflect_apply(function: JsValue, this: JsValue, arguments: JsValue) -> JsValue;
    #[js(new = "Object")] pub fn object() -> JsValue;
    #[js(global, get = "globalThis")] fn global_object() -> JsValue;
    #[js(new = "Array")] pub fn array() -> JsValue;
    #[js(index)] pub fn at(target: JsValue, index: usize) -> JsValue;
    #[js(get = "length")] pub fn length(target: JsValue) -> usize;
    #[js(method = "push")] pub fn push(target: JsValue, value: JsValue) -> usize;
    #[js(call = "Object.keys")] pub fn keys(target: JsValue) -> JsValue;
    #[js(call = "Array.isArray")] pub fn is_array(value: JsValue) -> bool;
    #[js(call = "Number.isInteger")] pub fn is_integer(value: JsValue) -> bool;
    #[js(call = "JSON.stringify")] fn stringify(value: JsValue) -> JsValue;
    #[js(call = "JSON.parse")] fn parse(text: JsValue) -> JsValue;
    #[js(call = "Number")] pub fn to_number(text: JsValue) -> f64;
    #[js(call = "Date.now")] pub fn now() -> f64;
    #[js(call = "Math.max")] pub fn math_max(a: f64, b: f64) -> f64;
    #[js(call = "Math.min")] pub fn math_min(a: f64, b: f64) -> f64;
    #[js(call = "Math.pow")] pub fn math_pow(base: f64, exponent: f64) -> f64;
    #[js(call = "setTimeout")] pub fn set_timeout(callback: JsValue, milliseconds: f64) -> JsValue;
    #[js(call = "clearTimeout")] pub fn clear_timeout(id: JsValue);
    #[js(new = "Promise")] pub fn promise(executor: JsValue) -> JsValue;
    #[js(method = "then")] pub fn then(promise: JsValue, fulfilled: JsValue, rejected: JsValue) -> JsValue;
    #[js(new = "Error")] fn error(message: JsValue) -> JsValue;
    #[js(new = "TypeError")] fn type_error(message: JsValue) -> JsValue;
    #[js(new = "RangeError")] fn range_error(message: JsValue) -> JsValue;

    #[js(new = "AbortController")] fn abort_controller() -> JsValue;
    #[js(get = "signal")] fn controller_signal(controller: JsValue) -> JsValue;
    #[js(method = "abort")] fn abort(controller: JsValue);
    #[js(method = "abort")] fn abort_with(controller: JsValue, reason: JsValue);
    #[js(call = "AbortSignal.any")] fn any_signal(signals: JsValue) -> JsValue;
    #[js(get = "aborted")] fn signal_aborted(signal: JsValue) -> bool;
    #[js(get = "reason")] fn signal_reason(signal: JsValue) -> JsValue;
    #[js(method = "throwIfAborted")] fn throw_if_aborted(signal: JsValue);
    #[js(method = "addEventListener")]
    fn add_event_listener(target: JsValue, kind: JsValue, listener: JsValue, options: JsValue);

    #[js(new = "Headers")] fn new_headers() -> JsValue;
    #[js(method = "set")] fn headers_set(headers: JsValue, name: JsValue, value: JsValue);
    #[js(method = "get")] fn headers_get(headers: JsValue, name: JsValue) -> JsValue;

    #[js(new = "Request")] fn new_request(input: JsValue, init: JsValue) -> JsValue;
    #[js(method = "clone")] fn clone_request(request: JsValue) -> JsValue;
    #[js(get = "method")] fn request_method(request: JsValue) -> JsValue;
    #[js(get = "url")] fn request_url(request: JsValue) -> JsValue;
    #[js(get = "headers")] fn message_headers(message: JsValue) -> JsValue;
    #[js(get = "signal")] fn request_signal(request: JsValue) -> JsValue;
    #[js(method = "text")] fn body_text(message: JsValue) -> JsValue;

    #[js(new = "Response")] fn new_response(body: JsValue, init: JsValue) -> JsValue;
    #[js(get = "status")] fn response_status(response: JsValue) -> u32;
    #[js(get = "ok")] fn response_ok(response: JsValue) -> bool;
    #[js(get = "body")] fn response_body(response: JsValue) -> JsValue;
    #[js(method = "arrayBuffer")] fn array_buffer(response: JsValue) -> JsValue;
    #[js(get = "byteLength")] fn byte_length(buffer: JsValue) -> usize;

    #[js(new = "TextDecoder")] fn utf8_decoder() -> JsValue;
    #[js(new = "TextDecoder")] fn labelled_decoder(label: JsValue) -> JsValue;
    #[js(method = "decode")] fn decode(decoder: JsValue, bytes: JsValue) -> JsValue;
}

pub fn s(text: &str) -> JsValue {
    JsValue::erase(text)
}

pub fn text(value: JsValue) -> &'static str {
    value.read::<&str>()
}

pub fn field(target: JsValue, key: &str) -> JsValue {
    property(target, s(key))
}

pub fn set_field(target: JsValue, key: &str, value: JsValue) {
    assign(target, s(key), value);
}

/// A JS array of `values`.
pub fn list(values: &[JsValue]) -> JsValue {
    let result = array();
    for &value in values {
        push(result, value);
    }
    result
}

/// A property of `target` that is present (not `undefined`).
pub fn present(target: JsValue, key: &str) -> Option<JsValue> {
    let value = field(target, key);
    (!value.is_undefined()).then_some(value)
}

/// Ky's `isError`: an Error of this realm, or one of another realm by its internal brand
/// (`Object.prototype.toString`).
pub fn is_error(value: JsValue) -> bool {
    let to_string = field(field(field(global_object(), "Object"), "prototype"), "toString");
    value.is_error() || text(reflect_apply(to_string, value, array())) == "[object Error]"
}

/// The `name` of a thrown value: an Error's own name, as the adapter reads `error.name`.
pub fn error_name(value: JsValue) -> &'static str {
    let name = field(value, "name");
    if name.is_string() { text(name) } else { "Error" }
}

pub fn new_error(message: &str) -> JsValue {
    error(s(message))
}

pub fn new_type_error(message: &str) -> JsValue {
    type_error(s(message))
}

pub fn new_range_error(message: &str) -> JsValue {
    range_error(s(message))
}

pub fn json_stringify(value: JsValue) -> &'static str {
    text(stringify(value))
}

/// `JSON.parse`, or the SyntaxError it throws.
pub fn json_parse(text: &str) -> Result<JsValue, JsValue> {
    try_host(|| parse(s(text)))
}

pub async fn sleep(milliseconds: f64) {
    let wait = promise(callback2(move |resolve, _| {
        set_timeout(resolve, milliseconds);
        JsValue::undefined()
    }));
    // A timer never rejects.
    let _ = await_promise(wait).await;
}

#[derive(Clone, Copy)]
pub struct AbortSignal(JsValue);

impl AbortSignal {
    /// `AbortSignal.any`: aborted as soon as one of `signals` is, with that one's reason.
    pub fn any(signals: &[AbortSignal]) -> AbortSignal {
        let sources = array();
        for signal in signals {
            push(sources, signal.0);
        }
        AbortSignal(any_signal(sources))
    }

    pub fn aborted(self) -> bool {
        signal_aborted(self.0)
    }

    pub fn reason(self) -> JsValue {
        signal_reason(self.0)
    }

    /// `throwIfAborted`: the reason, as the error it throws.
    pub fn throw_if_aborted(self) -> Result<(), JsValue> {
        try_host(|| {
            throw_if_aborted(self.0);
            JsValue::undefined()
        })
        .map(|_| ())
    }

    /// Calls `listener` once, when the signal aborts.
    pub fn on_abort(self, listener: JsValue) {
        let once = object();
        set_field(once, "once", JsValue::erase(true));
        add_event_listener(self.0, s("abort"), listener, once);
    }
}

#[derive(Clone, Copy)]
pub struct AbortController(JsValue);

impl AbortController {
    pub fn new() -> AbortController {
        AbortController(abort_controller())
    }

    pub fn signal(self) -> AbortSignal {
        AbortSignal(controller_signal(self.0))
    }

    /// `abort()`, whose reason is the platform's AbortError DOMException.
    pub fn abort(self) {
        abort(self.0);
    }

    pub fn abort_with(self, reason: JsValue) {
        abort_with(self.0, reason);
    }
}

#[derive(Clone, Copy)]
pub struct Headers(JsValue);

impl Headers {
    pub fn new() -> Headers {
        Headers(new_headers())
    }

    /// `Headers.set`: it validates the name and the value (a TypeError) and stores the value's
    /// ByteString conversion, so any JSON value is accepted as JS accepts it.
    pub fn set(self, name: &str, value: JsValue) -> Result<(), JsValue> {
        try_host(|| {
            headers_set(self.0, s(name), value);
            JsValue::undefined()
        })
        .map(|_| ())
    }

    pub fn get(self, name: &str) -> Option<&'static str> {
        let value = headers_get(self.0, s(name));
        (!value.is_null()).then(|| text(value))
    }
}

#[derive(Clone, Copy)]
pub struct Request(JsValue);

impl Request {
    /// `new Request(input, {method, headers, body, signal})`, or the TypeError it throws for an
    /// invalid URL or method, or a body on a GET or HEAD request.
    pub fn new(
        input: &str,
        method: &str,
        headers: Headers,
        body: Option<&str>,
        signal: AbortSignal,
    ) -> Result<Request, JsValue> {
        let init = object();
        set_field(init, "method", s(method));
        set_field(init, "headers", headers.0);
        if let Some(body) = body {
            set_field(init, "body", s(body));
        }
        set_field(init, "signal", signal.0);
        try_host(|| new_request(s(input), init)).map(Request)
    }

    /// `new Request(request, {signal})`: the same request, following another signal.
    pub fn with_signal(self, signal: AbortSignal) -> Result<Request, JsValue> {
        let init = object();
        set_field(init, "signal", signal.0);
        try_host(|| new_request(self.0, init)).map(Request)
    }

    pub fn clone(self) -> Result<Request, JsValue> {
        try_host(|| clone_request(self.0)).map(Request)
    }

    pub fn method(self) -> &'static str {
        text(request_method(self.0))
    }

    pub fn url(self) -> &'static str {
        text(request_url(self.0))
    }

    pub fn headers(self) -> Headers {
        Headers(message_headers(self.0))
    }

    pub fn signal(self) -> AbortSignal {
        AbortSignal(request_signal(self.0))
    }

    pub async fn text(self) -> Result<&'static str, JsValue> {
        await_promise(body_text(self.0)).await.map(text)
    }
}

#[derive(Clone, Copy)]
pub struct Response(JsValue);

impl Response {
    /// `new Response(body, {status, headers})`, or the RangeError or TypeError it throws.
    pub fn new(body: JsValue, status: JsValue, headers: JsValue) -> Result<Response, JsValue> {
        let init = object();
        set_field(init, "status", status);
        set_field(init, "headers", headers);
        try_host(|| new_response(body, init)).map(Response)
    }

    /// A Response a transport resolved with.
    pub fn from_value(value: JsValue) -> Response {
        Response(value)
    }

    pub fn as_value(self) -> JsValue {
        self.0
    }

    pub fn status(self) -> u32 {
        response_status(self.0)
    }

    pub fn ok(self) -> bool {
        response_ok(self.0)
    }

    pub fn headers(self) -> Headers {
        Headers(message_headers(self.0))
    }

    pub fn has_body(self) -> bool {
        !response_body(self.0).is_null()
    }

    pub async fn text(self) -> Result<&'static str, JsValue> {
        await_promise(body_text(self.0)).await.map(text)
    }

    /// The whole body, as an ArrayBuffer and its length.
    pub async fn bytes(self) -> Result<(JsValue, usize), JsValue> {
        let buffer = await_promise(array_buffer(self.0)).await?;
        Ok((buffer, byte_length(buffer)))
    }
}

#[derive(Clone, Copy)]
pub struct TextDecoder(JsValue);

impl TextDecoder {
    pub fn utf8() -> TextDecoder {
        TextDecoder(utf8_decoder())
    }

    /// A decoder for an Encoding Standard label, or the RangeError for an unknown one.
    pub fn new(label: &str) -> Result<TextDecoder, JsValue> {
        try_host(|| labelled_decoder(s(label))).map(TextDecoder)
    }

    pub fn decode(self, bytes: JsValue) -> &'static str {
        text(decode(self.0, bytes))
    }
}
