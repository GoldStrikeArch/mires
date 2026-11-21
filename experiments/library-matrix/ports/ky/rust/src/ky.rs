//! Ky 2.2.0 (`core/Ky.js`, `core/constants.js`, `utils/normalize.js`, `utils/merge.js`,
//! `utils/timeout.js`, `utils/is-network-error.js` and `errors/*.js`), the subset in
//! `libraries/ky/CONTRACT.md`.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;
use mires_js::{JsValue, await_promise, callback0, callback1, callback2, invoke};

use crate::host::{
    self, AbortController, AbortSignal, Headers, Request, Response, TextDecoder, has_property,
    is_array, is_integer, json_parse, json_stringify, keys, math_min, new_error, new_range_error,
    new_type_error, text,
};
use crate::retry_timing::{calculate_retry_timing_delay, get_retry_timing_header};

/// What a Ky call failed with: Ky's own errors (`errors/*.js`), or what a host API, the
/// transport or one of Ky's option checks threw (a TypeError, RangeError, SyntaxError or abort
/// reason), as the value it is. Ky's errors carry the request that failed, and a NetworkError its
/// cause, as Ky's do; the adapter reads only the name, the status and the data.
#[allow(dead_code)]
pub enum Failure {
    Http { response: Response, request: Request, data: Option<JsValue> },
    Network { request: Request, cause: JsValue },
    Timeout { request: Request },
    Thrown(JsValue),
}

impl Failure {
    pub fn name(&self) -> &'static str {
        match self {
            Failure::Http { .. } => "HTTPError",
            Failure::Network { .. } => "NetworkError",
            Failure::Timeout { .. } => "TimeoutError",
            Failure::Thrown(value) => host::error_name(*value),
        }
    }
}

impl From<JsValue> for Failure {
    fn from(value: JsValue) -> Failure {
        Failure::Thrown(value)
    }
}

/// The callbacks a caller hands Ky: the `fetch` option, `retry.delay` and the five hook kinds.
/// Ky takes a list of hooks of each kind; a caller here registers one of each.
pub trait Callbacks {
    /// The transport: a promise of the Response for `request`.
    fn fetch(&self, request: Request) -> JsValue;
    /// `retry.delay`: the backoff before retry `attempt_count` (one-based).
    fn delay(&self, attempt_count: u32) -> f64;
    fn init(&self);
    fn before_request(&self, request: Request, retry_count: u32);
    fn after_response(&self, request: Request, response: Response, retry_count: u32);
    fn before_retry(&self, request: Request, error: &Failure, retry_count: u32);
    fn before_error(&self, request: Request, error: &Failure, retry_count: u32);
}

/// The caller's retry options, as the JS values given (`None` where a value is undefined);
/// `normalize_retry_options` validates them.
pub struct RetryOptions {
    pub limit: Option<JsValue>,
    pub methods: Option<JsValue>,
    pub status_codes: Option<JsValue>,
    pub max_retry_after: Option<JsValue>,
    pub backoff_limit: Option<JsValue>,
    pub retry_on_timeout: Option<JsValue>,
}

pub struct Options<C: Callbacks> {
    pub method: Option<&'static str>,
    /// A plain object of header names to values.
    pub headers: Option<JsValue>,
    pub json: Option<JsValue>,
    pub retry: RetryOptions,
    pub timeout: Option<JsValue>,
    pub throw_http_errors: Option<JsValue>,
    pub signal: Option<AbortSignal>,
    pub callbacks: C,
}

struct Retry {
    limit: f64,
    methods: Vec<String>,
    status_codes: Vec<f64>,
    after_status_codes: [f64; 3],
    max_retry_after: f64,
    backoff_limit: f64,
    retry_on_timeout: bool,
}

const REQUEST_METHODS: [&str; 7] = ["get", "post", "put", "patch", "head", "delete", "query"];
const CASE_INSENSITIVE_METHODS: [&str; 9] =
    ["get", "post", "put", "patch", "head", "delete", "query", "options", "trace"];
const RETRY_METHODS: [&str; 7] = ["get", "put", "head", "delete", "options", "trace", "query"];
const RETRY_STATUS_CODES: [f64; 7] = [408.0, 413.0, 429.0, 500.0, 502.0, 503.0, 504.0];
const RETRY_AFTER_STATUS_CODES: [f64; 3] = [413.0, 429.0, 503.0];
/// The maximum value of a 32bit int (see issue #117)
const MAX_SAFE_TIMEOUT: f64 = 2_147_483_647.0;
const MAX_ERROR_RESPONSE_BODY_SIZE: usize = 10 * 1024 * 1024;

/// `normalizeRequestMethod`, with JS's full case mapping (Rust's `to_lowercase` and
/// `to_uppercase` apply the same Unicode mappings as `toLowerCase` and `toUpperCase`).
fn normalize_request_method(input: &str) -> String {
    if REQUEST_METHODS.contains(&input.to_lowercase().as_str()) {
        input.to_uppercase()
    } else {
        String::from(input)
    }
}

/// `normalizeRetryMethod`: common methods for retry matching are case-insensitive, custom ones
/// keep their case.
fn normalize_retry_method(method: &str) -> String {
    let lowercase = method.to_lowercase();
    if CASE_INSENSITIVE_METHODS.contains(&lowercase.as_str()) {
        lowercase
    } else {
        String::from(method)
    }
}

fn non_negative(value: JsValue) -> Option<f64> {
    if value.is_number() {
        let number = value.read::<f64>();
        (number >= 0.0).then_some(number)
    } else {
        None
    }
}

/// An array option: the default when undefined, or `Error` when not an array.
fn array_option(value: Option<JsValue>, message: &str) -> Result<Option<Vec<JsValue>>, Failure> {
    match value {
        None => Ok(None),
        Some(value) if is_array(value) => {
            Ok(Some((0..host::length(value)).map(|index| host::at(value, index)).collect()))
        }
        Some(_) => Err(Failure::Thrown(new_error(message))),
    }
}

fn limit_option(value: Option<JsValue>, message: &str) -> Result<f64, Failure> {
    match value {
        None => Ok(f64::INFINITY),
        Some(value) => non_negative(value).ok_or_else(|| Failure::Thrown(new_type_error(message))),
    }
}

/// `normalizeRetryOptions`: the defaults for undefined values, and Ky's checks, in its order.
fn normalize_retry_options(retry: &RetryOptions) -> Result<Retry, Failure> {
    let limit = match retry.limit {
        None => 2.0,
        Some(limit) if is_integer(limit) && limit.read::<f64>() >= 0.0 => limit.read::<f64>(),
        Some(_) => {
            return Err(Failure::Thrown(new_type_error(
                "`retry.limit` must be a finite, non-negative integer",
            )));
        }
    };
    let methods = array_option(retry.methods, "retry.methods must be an array")?;
    let status_codes = array_option(retry.status_codes, "retry.statusCodes must be an array")?;
    // A mistyped entry only ever fails to match, so it is reported instead.
    let methods = match methods {
        None => RETRY_METHODS.iter().map(|method| String::from(*method)).collect(),
        Some(values) => {
            let mut methods = Vec::with_capacity(values.len());
            for value in values {
                if !value.is_string() {
                    return Err(Failure::Thrown(new_type_error(
                        "`retry.methods` must only contain strings",
                    )));
                }
                methods.push(normalize_retry_method(text(value)));
            }
            methods
        }
    };
    let status_codes = match status_codes {
        None => RETRY_STATUS_CODES.to_vec(),
        Some(values) => {
            let mut status_codes = Vec::with_capacity(values.len());
            for value in values {
                if !is_integer(value) {
                    return Err(Failure::Thrown(new_type_error(
                        "`retry.statusCodes` must only contain numbers",
                    )));
                }
                status_codes.push(value.read::<f64>());
            }
            status_codes
        }
    };
    let max_retry_after = limit_option(
        retry.max_retry_after,
        "`retry.maxRetryAfter` must be a non-negative number or `Infinity`",
    )?;
    let backoff_limit = limit_option(
        retry.backoff_limit,
        "`retry.backoffLimit` must be a non-negative number or `Infinity`",
    )?;
    let retry_on_timeout = match retry.retry_on_timeout {
        None => false,
        Some(value) if value.is_boolean() => value.read::<bool>(),
        Some(_) => {
            return Err(Failure::Thrown(new_type_error("`retry.retryOnTimeout` must be a boolean")));
        }
    };
    Ok(Retry {
        limit,
        methods,
        status_codes,
        after_status_codes: RETRY_AFTER_STATUS_CODES,
        max_retry_after,
        backoff_limit,
        retry_on_timeout,
    })
}

/// `validateTimeoutOption`: milliseconds or `false` (`None`).
fn validate_timeout_option(value: JsValue) -> Result<Option<f64>, Failure> {
    if value.is_boolean() && !value.read::<bool>() {
        return Ok(None);
    }
    match non_negative(value) {
        // `Infinity` lands here too, since it is not a usable delay either.
        Some(milliseconds) if milliseconds > MAX_SAFE_TIMEOUT => Err(Failure::Thrown(
            new_range_error("The `timeout` option cannot be greater than 2147483647"),
        )),
        Some(milliseconds) => Ok(Some(milliseconds)),
        None => Err(Failure::Thrown(new_type_error(
            "The `timeout` option must be a non-negative number or `false`",
        ))),
    }
}

/// Option merging (`deepMerge`, `mergeHeaderContainers`) keys the headers by their lowercased
/// name, the last spelling of a name winning at the place of its first. Rust's `to_lowercase`
/// applies the same full Unicode mapping as JS `toLowerCase` (U+212A KELVIN SIGN becomes `k`).
fn lowercase_headers(headers: JsValue) -> Vec<(String, JsValue)> {
    let names = keys(headers);
    let mut merged: Vec<(String, JsValue)> = Vec::new();
    for index in 0..host::length(names) {
        let name = host::at(names, index);
        let value = host::property(headers, name);
        let name = text(name).to_lowercase();
        match merged.iter_mut().find(|(known, _)| *known == name) {
            Some(entry) => entry.1 = value,
            None => merged.push((name, value)),
        }
    }
    merged
}

/// `mergeHeaders`: each header set on a fresh Headers, which validates it.
fn merge_headers(headers: &[(String, JsValue)]) -> Result<Headers, Failure> {
    let result = Headers::new();
    for (name, value) in headers {
        result.set(name, *value)?;
    }
    Ok(result)
}

const NETWORK_ERROR_MESSAGES: [&str; 8] = [
    "network error",
    "NetworkError when attempting to fetch resource.",
    "The Internet connection appears to be offline.",
    "Network request failed",
    "fetch failed",
    "terminated",
    " A network error occurred.",
    "Network connection lost",
];

/// `utils/is-network-error.js`, inlined by Ky from is-network-error 1.3.2.
fn is_raw_network_error(error: JsValue) -> bool {
    if !host::is_error(error) || host::error_name(error) != "TypeError" {
        return false;
    }
    let message = host::field(error, "message");
    if !message.is_string() {
        return false;
    }
    let message = text(message);
    let wrapped = |prefix: &str| message.starts_with(prefix) && message.ends_with(')');
    if message == "Load failed" || wrapped("Load failed (") {
        // Safari 17+ has generic message but no stack for network errors
        return host::field(error, "stack").is_undefined()
            || has_property(error, host::s("__sentry_captured__"));
    }
    message.starts_with("error sending request for url")
        || message == "Failed to fetch"
        || wrapped("Failed to fetch (")
        || NETWORK_ERROR_MESSAGES.contains(&message)
}

/// JS `String.prototype.trim`: WhiteSpace and LineTerminator code points at either end.
fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// `\s` in a JS regular expression, the code points `trim` removes.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// Ky's `isJsonContentType`: JSON subtypes like `json`, `problem+json` and `vnd.api+json`,
/// written out rather than matched with a pattern, as Ky does.
fn is_json_content_type(content_type: &str) -> bool {
    let first = content_type.split(';').next().unwrap_or("");
    let mime_type = js_trim(first).to_lowercase();
    let Some(subtype) = mime_type.strip_suffix("json") else {
        return false;
    };
    // `mimeType.at(-5)`: the character before `json`, which is ASCII whenever it matters.
    match subtype.bytes().last() {
        Some(b'/') => true,
        // The separator is not the slash, so any slash in the type comes before it.
        Some(b'.' | b'+' | b'-') => mime_type.contains('/'),
        _ => false,
    }
}

/// The charset parameter, as `/;\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))/i` finds it: after the
/// first `;` where the pattern matches, a quoted value of one or more characters, or else a run
/// of characters other than `;`, `,` and whitespace.
fn charset(content_type: &str) -> Option<&str> {
    let mut rest = content_type;
    while let Some(index) = rest.find(';') {
        rest = &rest[index + 1..];
        let after = rest.trim_start_matches(is_js_space);
        let Some(name) = after.get(..7) else { continue };
        if !name.eq_ignore_ascii_case("charset") {
            continue;
        }
        let after = after[7..].trim_start_matches(is_js_space);
        let Some(after) = after.strip_prefix('=') else { continue };
        let value = after.trim_start_matches(is_js_space);
        if let Some(quoted) = value.strip_prefix('"') {
            if let Some(end) = quoted.find('"').filter(|end| *end > 0) {
                return Some(&quoted[..end]);
            }
        }
        let end = value.find(|c| c == ';' || c == ',' || is_js_space(c)).unwrap_or(value.len());
        if end > 0 {
            return Some(&value[..end]);
        }
    }
    None
}

/// `createTextDecoder`: the charset's decoder, or UTF-8 for none or an unknown label.
fn create_text_decoder(content_type: &str) -> TextDecoder {
    match charset(content_type) {
        Some(label) => TextDecoder::new(label).unwrap_or_else(|_| TextDecoder::utf8()),
        None => TextDecoder::utf8(),
    }
}

/// `readResponseText`: an error body as text, or `None` when it cannot be read or is larger
/// than `maxErrorResponseBodySize`. JSON uses UTF-8 regardless of the charset (RFC 8259).
async fn read_response_text(response: Response) -> Option<&'static str> {
    if !response.has_body() {
        return response.text().await.ok();
    }
    let content_type = response.headers().get("content-type").unwrap_or("");
    let decoder = if is_json_content_type(content_type) {
        TextDecoder::utf8()
    } else {
        create_text_decoder(content_type)
    };
    let (bytes, length) = response.bytes().await.ok()?;
    (length <= MAX_ERROR_RESPONSE_BODY_SIZE).then(|| decoder.decode(bytes))
}

/// `#getResponseData`: the parsed JSON or the text of an error body, or `None` when it is empty,
/// unreadable or unparsable.
async fn get_response_data(response: Response) -> Option<JsValue> {
    let text = read_response_text(response).await.filter(|text| !text.is_empty())?;
    if !is_json_content_type(response.headers().get("content-type").unwrap_or("")) {
        return Some(host::s(text));
    }
    json_parse(text).ok()
}

/// `utils/timeout.js`: the transport raced against a timer that aborts `controller` and rejects.
/// `timed_out` is Ky's own flag: once it is set, the outcome is the TimeoutError, and a response
/// arriving later is discarded.
fn timeout<C: Callbacks>(
    callbacks: &C,
    request: Request,
    controller: AbortController,
    milliseconds: f64,
    timed_out: Rc<Cell<bool>>,
) -> JsValue {
    let settle = Rc::new(Cell::new((JsValue::undefined(), JsValue::undefined())));
    let timer = Rc::new(Cell::new(JsValue::undefined()));
    let race = {
        let (settle, timer, timed_out) = (settle.clone(), timer.clone(), timed_out.clone());
        host::promise(callback2(move |resolve, reject| {
            settle.set((resolve, reject));
            let timed_out = timed_out.clone();
            let expire = callback0(move || {
                timed_out.set(true);
                controller.abort();
                let _ = invoke(reject, JsValue::undefined(), &[JsValue::undefined()]);
                JsValue::undefined()
            });
            timer.set(host::set_timeout(expire, milliseconds));
            JsValue::undefined()
        }))
    };
    let (resolve, reject) = settle.get();
    let fulfilled = callback1(move |response| {
        if !timed_out.get() {
            let _ = invoke(resolve, JsValue::undefined(), &[response]);
        }
        JsValue::undefined()
    });
    let rejected = callback1(move |error| invoke(reject, JsValue::undefined(), &[error]));
    host::then(callbacks.fetch(request), fulfilled, rejected);
    let clear = callback0(move || {
        host::clear_timeout(timer.get());
        JsValue::undefined()
    });
    host::then(race, clear, clear);
    race
}

pub struct Ky<C: Callbacks> {
    request: Request,
    abort_controller: AbortController,
    /// `#options.signal`: the user's signal and the abort controller's, combined.
    signal: AbortSignal,
    retry_count: u32,
    original_request: Option<Request>,
    user_provided_abort_signal: Option<AbortSignal>,
    retry: Retry,
    timeout: Option<f64>,
    throw_http_errors: bool,
    callbacks: C,
}

fn create_managed_signal(user: Option<AbortSignal>, controller: AbortController) -> AbortSignal {
    match user {
        Some(user) => AbortSignal::any(&[user, controller.signal()]),
        None => controller.signal(),
    }
}

/// The body shortcuts, and the media type each one advertises.
#[derive(Clone, Copy, PartialEq)]
pub enum Shortcut {
    Json,
    Text,
}

impl Shortcut {
    fn mime_type(self) -> &'static str {
        match self {
            Shortcut::Json => "application/json",
            Shortcut::Text => "text/*",
        }
    }
}

/// `ky(input, options)[shortcut]()`: the init hooks, the request, the run and the body read. A
/// construction error fails without running the beforeError hooks, and so does a body that is
/// not JSON.
pub async fn request<C: Callbacks>(
    input: &str,
    options: Options<C>,
    shortcut: Shortcut,
) -> Result<JsValue, Failure> {
    options.callbacks.init();
    let mut ky = Ky::new(input, options)?;
    // Before dispatch, `ky.request` is the request that will be sent.
    let headers = ky.request.headers();
    let accept = headers.get("accept").filter(|accept| !accept.is_empty());
    headers.set("accept", host::s(accept.unwrap_or(shortcut.mime_type())))?;
    let response = match ky.run().await {
        Ok(response) => response,
        Err(error) => return Err(ky.throw_processed_error(error)),
    };
    let body = response.text().await?;
    match shortcut {
        Shortcut::Text => Ok(host::s(body)),
        Shortcut::Json => Ok(json_parse(body)?),
    }
}

impl<C: Callbacks> Ky<C> {
    fn new(input: &str, options: Options<C>) -> Result<Ky<C>, Failure> {
        let headers = options.headers.map(lowercase_headers).unwrap_or_default();
        let request_headers = merge_headers(&headers)?;
        let method = normalize_request_method(options.method.unwrap_or("GET"));
        let retry = normalize_retry_options(&options.retry)?;
        // Defaults only replace `undefined`, so a `null` reaches the validation and is reported.
        let throw_http_errors = match options.throw_http_errors {
            None => true,
            Some(value) if value.is_boolean() => value.read::<bool>(),
            Some(_) => {
                return Err(Failure::Thrown(new_type_error(
                    "The `throwHttpErrors` option must be a boolean or a function",
                )));
            }
        };
        let timeout = match options.timeout {
            None => Some(10_000.0),
            Some(value) => validate_timeout_option(value)?,
        };
        let abort_controller = AbortController::new();
        let signal = create_managed_signal(options.signal, abort_controller);
        // A content-type from `options.headers` counts as user-provided.
        let user_provided_content_type = headers.iter().any(|(name, _)| name == "content-type");
        let body = options.json.map(json_stringify);
        if body.is_some() && !user_provided_content_type {
            request_headers.set("content-type", host::s("application/json"))?;
        }
        let request = Request::new(input, &method, request_headers, body, signal)?;
        Ok(Ky {
            request,
            abort_controller,
            signal,
            retry_count: 0,
            original_request: None,
            user_provided_abort_signal: options.signal,
            retry,
            timeout,
            throw_http_errors,
            callbacks: options.callbacks,
        })
    }

    /// `#sentRequest`: `fetch` replaces `request` with the clone it prepares for a possible
    /// retry, so the request that was actually sent is reported.
    fn sent_request(&self) -> Request {
        self.original_request.unwrap_or(self.request)
    }

    /// Chromium reports a user abort during a fetch as a network error; the abort reason is
    /// surfaced instead.
    fn throw_if_aborted_by_user(&self) -> Result<(), Failure> {
        match self.user_provided_abort_signal {
            Some(signal) => Ok(signal.throw_if_aborted()?),
            None => Ok(()),
        }
    }

    async fn run(&mut self) -> Result<Response, Failure> {
        // A hook runs inside `#raceWithTotalTimeout()`, which first throws a user abort.
        self.throw_if_aborted_by_user()?;
        self.callbacks.before_request(self.request, 0);
        let mut response = self.retry(None).await?;
        loop {
            self.throw_if_aborted_by_user()?;
            self.callbacks.after_response(self.request, response, self.retry_count);
            if response.ok() || !self.throw_http_errors {
                return Ok(response);
            }
            let data = get_response_data(response).await;
            let error = Failure::Http { response, request: self.sent_request(), data };
            self.throw_if_aborted_by_user()?;
            response = self.retry(Some(error)).await?;
        }
    }

    /// `#retry()` and `#retryFromError()`, which call each other in Ky, as one loop: each pending
    /// error either is retried (the wait, the beforeRetry hooks and the next fetch) or ends it.
    async fn retry(&mut self, mut pending: Option<Failure>) -> Result<Response, Failure> {
        loop {
            if let Some(error) = pending.take() {
                self.prepare_retry(error).await?;
            }
            match self.fetch().await {
                Ok(response) => return Ok(response),
                Err(error) => pending = Some(error),
            }
        }
    }

    /// `#retryFromError()` up to its final `#retry()`.
    async fn prepare_retry(&mut self, error: Failure) -> Result<(), Failure> {
        self.throw_if_aborted_by_user()?;
        let Some(delay) = self.calculate_retry_delay(&error)? else {
            return Err(error);
        };
        host::sleep(math_min(delay, MAX_SAFE_TIMEOUT)).await;
        // Reset the abort controller if it was aborted (on a timeout retry), so hooks and the
        // retried request get a fresh signal.
        if self.abort_controller.signal().aborted() {
            self.abort_controller = AbortController::new();
            self.signal = create_managed_signal(self.user_provided_abort_signal, self.abort_controller);
            self.request = self.request.with_signal(self.signal)?;
        }
        self.throw_if_aborted_by_user()?;
        self.callbacks.before_retry(self.request, &error, self.retry_count + 1);
        self.retry_count += 1;
        Ok(())
    }

    async fn fetch(&mut self) -> Result<Response, Failure> {
        // Cloning is done here to prepare in advance for retries, and skipped when retries are
        // disabled.
        let retry_request = if self.retry.limit > 0.0 {
            Some(self.request.clone()?.with_signal(self.signal)?)
        } else {
            None
        };
        let request = self.request;
        self.original_request = Some(request);
        if let Some(retry_request) = retry_request {
            self.request = retry_request;
        }
        let timed_out = Rc::new(Cell::new(false));
        let sent = match self.timeout {
            None => self.callbacks.fetch(request),
            Some(milliseconds) => {
                timeout(&self.callbacks, request, self.abort_controller, milliseconds, timed_out.clone())
            }
        };
        match await_promise(sent).await {
            Ok(response) => Ok(Response::from_value(response)),
            Err(error) => {
                self.throw_if_aborted_by_user()?;
                if timed_out.get() {
                    Err(Failure::Timeout { request })
                } else if is_raw_network_error(error) {
                    Err(Failure::Network { request, cause: error })
                } else {
                    Err(Failure::Thrown(error))
                }
            }
        }
    }

    /// `#calculateDelay()`: the `retry.delay` backoff, capped by `retry.backoffLimit`.
    fn calculate_delay(&self) -> Result<f64, Failure> {
        let retry_delay = self.callbacks.delay(self.retry_count + 1);
        if !(retry_delay >= 0.0) {
            return Err(Failure::Thrown(new_type_error(
                "`retry.delay` must return a non-negative number or `Infinity`",
            )));
        }
        Ok(math_min(self.retry.backoff_limit, retry_delay))
    }

    /// `#calculateRetryDelay()`: the wait before retrying `error`, or `None` when it is not
    /// retried and fails the call as it is.
    fn calculate_retry_delay(&self, error: &Failure) -> Result<Option<f64>, Failure> {
        if f64::from(self.retry_count) >= self.retry.limit {
            return Ok(None);
        }
        let method = normalize_retry_method(self.request.method());
        if !self.retry.methods.contains(&method) {
            return Ok(None);
        }
        match error {
            Failure::Timeout { .. } if self.retry.retry_on_timeout => self.calculate_delay().map(Some),
            Failure::Http { response, .. } => {
                let status = f64::from(response.status());
                if !self.retry.status_codes.contains(&status) {
                    return Ok(None);
                }
                match get_retry_timing_header(response.headers()) {
                    Some(header) if self.retry.after_status_codes.contains(&status) => {
                        match calculate_retry_timing_delay(&header) {
                            // Malformed retry timing headers should not disable retries; they
                            // only lose their server-provided timing.
                            None => self.calculate_delay().map(Some),
                            // No jitter when the server provides explicit retry timing.
                            Some(after) => Ok(Some(math_min(self.retry.max_retry_after, after))),
                        }
                    }
                    _ if status == 413.0 => Ok(None),
                    _ => self.calculate_delay().map(Some),
                }
            }
            Failure::Network { .. } => self.calculate_delay().map(Some),
            // Only known retriable error types are retried.
            _ => Ok(None),
        }
    }

    /// `#throwProcessedError()`: the beforeError hooks see the error, which then fails the call.
    fn throw_processed_error(&self, error: Failure) -> Failure {
        self.callbacks.before_error(self.sent_request(), &error, self.retry_count);
        error
    }
}
