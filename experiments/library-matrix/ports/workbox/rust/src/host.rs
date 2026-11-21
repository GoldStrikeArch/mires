//! The Web APIs Workbox runs on, as typed handles: Request, Response, URL, RegExp, CacheStorage,
//! Cache, fetch and timers. Every handle is the host object itself; a failed host call becomes
//! `Error::Host` with the thrown or rejected value.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use mires_js::{JsValue, await_promise, callback2, invoke, js_bindings};

use crate::workbox_core::Error;

js_bindings! {
    #[js(global, get = "location")] fn location() -> JsValue;
    #[js(global, get = "caches")] fn caches() -> JsValue;
    #[js(global, get = "crypto")] fn crypto() -> JsValue;
    #[js(method = "randomUUID")] fn crypto_random_uuid(crypto: JsValue) -> JsValue;
    #[js(call = "Reflect.get")] fn reflect_get(target: JsValue, key: JsValue) -> JsValue;
    #[js(call = "Reflect.set")] fn reflect_set(target: JsValue, key: JsValue, value: JsValue) -> bool;
    #[js(call = "Reflect.defineProperty")] fn reflect_define(target: JsValue, key: JsValue, descriptor: JsValue) -> bool;
    #[js(call = "Array.from")] fn array_from(iterable: JsValue) -> JsValue;
    #[js(call = "JSON.stringify")] fn json_stringify(value: JsValue) -> JsValue;
    #[js(call = "String")] pub fn string_of(value: JsValue) -> JsValue;
    #[js(call = "console.warn")] fn console_warn(message: JsValue);
    #[js(call = "setTimeout")] fn set_timeout(callback: JsValue, delay: f64) -> JsValue;
    #[js(call = "fetch")] fn host_fetch(request: JsValue) -> JsValue;
    #[js(new = "Object")] pub fn object() -> JsValue;
    #[js(new = "Array")] pub fn array() -> JsValue;
    #[js(new = "Promise")] fn new_promise(executor: JsValue) -> JsValue;
    #[js(new = "Error")] fn new_error(message: JsValue) -> JsValue;
    #[js(new = "TypeError")] fn new_type_error(message: JsValue) -> JsValue;
    #[js(new = "Request", fallible)] fn new_request(input: JsValue, init: JsValue) -> Result<JsValue, JsValue>;
    #[js(new = "Response", fallible)] fn new_response(body: JsValue, init: JsValue) -> Result<JsValue, JsValue>;
    #[js(new = "Headers", fallible)] fn new_headers(init: JsValue) -> Result<JsValue, JsValue>;
    #[js(new = "URL", fallible)] fn new_url(input: JsValue, base: JsValue) -> Result<JsValue, JsValue>;
    #[js(new = "URL", fallible)] fn new_absolute_url(input: JsValue) -> Result<JsValue, JsValue>;
    #[js(new = "RegExp", fallible)] fn new_regexp(source: JsValue) -> Result<JsValue, JsValue>;
    #[js(get = "length")] pub fn length(value: JsValue) -> usize;
    #[js(index)] pub fn at(value: JsValue, index: usize) -> JsValue;
    #[js(method = "push")] fn array_push(array: JsValue, value: JsValue) -> usize;
    #[js(method = "test")] fn regexp_test(regexp: JsValue, text: JsValue) -> bool;
    #[js(method = "clone")] fn clone_body(value: JsValue) -> JsValue;
    #[js(method = "text")] fn read_text(value: JsValue) -> JsValue;
    #[js(method = "blob")] fn read_blob(value: JsValue) -> JsValue;
    #[js(method = "keys")] fn search_keys(params: JsValue) -> JsValue;
    #[js(method = "set")] fn search_set(params: JsValue, name: JsValue, value: JsValue);
    #[js(method = "delete")] fn search_delete(params: JsValue, name: JsValue);
    #[js(method = "open")] fn storage_open(storage: JsValue, name: JsValue) -> JsValue;
    #[js(method = "match")] fn storage_match(storage: JsValue, request: JsValue, options: JsValue) -> JsValue;
    #[js(method = "keys")] fn storage_keys(storage: JsValue) -> JsValue;
    #[js(method = "delete")] fn storage_delete(storage: JsValue, name: JsValue) -> JsValue;
    #[js(method = "match")] fn cache_match(cache: JsValue, request: JsValue) -> JsValue;
    #[js(method = "put")] fn cache_put(cache: JsValue, request: JsValue, response: JsValue) -> JsValue;
    #[js(method = "delete")] fn cache_delete(cache: JsValue, request: JsValue) -> JsValue;
    #[js(method = "keys")] fn cache_keys(cache: JsValue) -> JsValue;
    #[js(method = "keys")] fn cache_keys_matching(cache: JsValue, request: JsValue, options: JsValue) -> JsValue;
    #[js(method = "waitUntil")] fn event_wait_until(event: JsValue, promise: JsValue);
}

pub fn text(value: &str) -> JsValue {
    JsValue::erase(value)
}

/// Reads a host string into Rust.
pub fn string(value: JsValue) -> String {
    value.read::<&str>().to_string()
}

pub fn get(target: JsValue, key: &str) -> JsValue {
    reflect_get(target, text(key))
}

pub fn set(target: JsValue, key: &str, value: JsValue) {
    reflect_set(target, text(key), value);
}

pub fn push(array: JsValue, value: JsValue) {
    array_push(array, value);
}

/// Defines a non-enumerable, read-only own data property, as `Object.defineProperty` does with
/// a `{value}` descriptor.
pub fn define_value(target: JsValue, key: &str, value: JsValue) {
    let descriptor = object();
    set(descriptor, "value", value);
    reflect_define(target, text(key), descriptor);
}

pub fn stringify(value: JsValue) -> String {
    string(json_stringify(value))
}

pub fn warn(message: &str) {
    console_warn(text(message));
}

pub fn type_error(message: &str) -> JsValue {
    new_type_error(text(message))
}

pub fn error(message: &str) -> JsValue {
    new_error(text(message))
}

pub fn random_uuid() -> String {
    string(crypto_random_uuid(crypto()))
}

pub fn location_href() -> String {
    string(get(location(), "href"))
}

pub fn location_origin() -> String {
    string(get(location(), "origin"))
}

/// Awaits a host promise, keeping its rejection as `Error::Host`.
pub async fn settle(promise: JsValue) -> Result<JsValue, Error> {
    await_promise(promise).await.map_err(Error::Host)
}

/// A promise resolved by a timer task (`setTimeout(resolve, ms)`), as workbox-core's `timeout`.
pub async fn timeout(ms: f64) {
    let executor = callback2(move |resolve, _| {
        set_timeout(resolve, ms);
        JsValue::undefined()
    });
    let _ = await_promise(new_promise(executor)).await;
}

/// A promise and the function that resolves it (workbox-core's `Deferred`).
pub struct Deferred {
    promise: JsValue,
    resolve: JsValue,
}

impl Deferred {
    pub fn new() -> Self {
        let resolve = alloc::rc::Rc::new(core::cell::Cell::new(JsValue::undefined()));
        let slot = alloc::rc::Rc::clone(&resolve);
        let promise = new_promise(callback2(move |resolve, _| {
            slot.set(resolve);
            JsValue::undefined()
        }));
        Deferred { promise, resolve: resolve.get() }
    }

    pub fn promise(&self) -> JsValue {
        self.promise
    }

    pub fn resolve(&self) {
        let _ = invoke(self.resolve, JsValue::undefined(), &[JsValue::null()]);
    }
}

/// The options of `new Request(input, init)` that Workbox and the adapter use; `None` leaves a
/// member out.
#[derive(Default)]
pub struct RequestInit<'a> {
    pub method: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub integrity: Option<&'a str>,
    pub cache: Option<&'a str>,
    pub credentials: Option<&'a str>,
    pub headers: Option<JsValue>,
}

impl RequestInit<'_> {
    fn value(&self) -> JsValue {
        let init = object();
        let members = [("method", self.method), ("mode", self.mode), ("integrity", self.integrity), ("cache", self.cache), ("credentials", self.credentials)];
        for (name, member) in members {
            if let Some(member) = member {
                set(init, name, text(member));
            }
        }
        if let Some(headers) = self.headers {
            set(init, "headers", headers);
        }
        init
    }
}

/// A Fetch `Request`.
#[derive(Clone, Copy)]
pub struct Request(JsValue);

impl Request {
    pub fn from_value(value: JsValue) -> Self {
        Request(value)
    }

    /// `new Request(url, init)`.
    pub fn new(url: &str, init: &RequestInit<'_>) -> Result<Self, Error> {
        new_request(text(url), init.value()).map(Request).map_err(Error::Host)
    }

    /// `new Request(request, init)`: a copy of this request with `init` applied.
    pub fn with(self, init: &RequestInit<'_>) -> Result<Self, Error> {
        new_request(self.0, init.value()).map(Request).map_err(Error::Host)
    }

    pub fn value(self) -> JsValue {
        self.0
    }

    pub fn url(self) -> String {
        string(get(self.0, "url"))
    }

    pub fn method(self) -> String {
        string(get(self.0, "method"))
    }

    pub fn mode(self) -> String {
        string(get(self.0, "mode"))
    }

    pub fn integrity(self) -> String {
        string(get(self.0, "integrity"))
    }

    pub fn headers(self) -> JsValue {
        get(self.0, "headers")
    }

    /// The host's `request.clone()`: a new Request with its own body.
    pub fn duplicate(self) -> Self {
        Request(clone_body(self.0))
    }
}

/// A Fetch `Response`.
#[derive(Clone, Copy)]
pub struct Response(JsValue);

impl Response {
    pub fn from_value(value: JsValue) -> Self {
        Response(value)
    }

    /// `new Response(body, {status})`.
    pub fn new(body: JsValue, status: JsValue) -> Result<Self, Error> {
        let init = object();
        set(init, "status", status);
        new_response(body, init).map(Response).map_err(Error::Host)
    }

    /// `new Response(body, {headers: new Headers(headers), status, statusText})`.
    pub fn with_parts(body: JsValue, headers: JsValue, status: u16, status_text: JsValue) -> Result<Self, Error> {
        let init = object();
        set(init, "headers", new_headers(headers).map_err(Error::Host)?);
        set(init, "status", JsValue::erase(f64::from(status)));
        set(init, "statusText", status_text);
        new_response(body, init).map(Response).map_err(Error::Host)
    }

    pub fn value(self) -> JsValue {
        self.0
    }

    pub fn status(self) -> u16 {
        get(self.0, "status").read::<f64>() as u16
    }

    pub fn status_text(self) -> JsValue {
        get(self.0, "statusText")
    }

    pub fn url(self) -> String {
        string(get(self.0, "url"))
    }

    pub fn redirected(self) -> bool {
        get(self.0, "redirected").is_truthy()
    }

    /// The response `type` ("basic", "cors", "error", …).
    pub fn kind(self) -> String {
        string(get(self.0, "type"))
    }

    pub fn headers(self) -> JsValue {
        get(self.0, "headers")
    }

    /// The host's `response.clone()`: a new Response with its own body.
    pub fn duplicate(self) -> Self {
        Response(clone_body(self.0))
    }

    /// `response.text()`, kept as the host string.
    pub async fn text(self) -> Result<JsValue, Error> {
        settle(read_text(self.0)).await
    }

    pub async fn blob(self) -> Result<JsValue, Error> {
        settle(read_blob(self.0)).await
    }
}

/// A WHATWG `URL`.
pub struct Url(JsValue);

impl Url {
    /// `new URL(input, base)`.
    pub fn parse(input: &str, base: &str) -> Result<Self, Error> {
        new_url(text(input), text(base)).map(Url).map_err(Error::Host)
    }

    /// `new URL(input)`, for an absolute URL.
    pub fn absolute(input: &str) -> Result<Self, Error> {
        new_absolute_url(text(input)).map(Url).map_err(Error::Host)
    }

    pub fn href(&self) -> String {
        string(get(self.0, "href"))
    }

    pub fn origin(&self) -> String {
        string(get(self.0, "origin"))
    }

    pub fn protocol(&self) -> String {
        string(get(self.0, "protocol"))
    }

    pub fn pathname(&self) -> String {
        string(get(self.0, "pathname"))
    }

    pub fn set_pathname(&self, pathname: &str) {
        set(self.0, "pathname", text(pathname));
    }

    pub fn set_hash(&self, hash: &str) {
        set(self.0, "hash", text(hash));
    }

    /// `[...url.searchParams.keys()]`.
    pub fn search_param_names(&self) -> Vec<String> {
        let names = array_from(search_keys(get(self.0, "searchParams")));
        (0..length(names)).map(|i| string(at(names, i))).collect()
    }

    /// `url.searchParams.set(name, value)`: the host converts `value` with ToString.
    pub fn set_search_param(&self, name: &str, value: JsValue) {
        search_set(get(self.0, "searchParams"), text(name), value);
    }

    pub fn delete_search_param(&self, name: &str) {
        search_delete(get(self.0, "searchParams"), text(name));
    }
}

/// A JavaScript `RegExp`, which keeps JavaScript's matching semantics (UTF-16 `.`, ASCII `\d`).
pub struct RegExp(JsValue);

impl RegExp {
    pub fn new(source: &str) -> Result<Self, Error> {
        new_regexp(text(source)).map(RegExp).map_err(Error::Host)
    }

    pub fn test(&self, input: &str) -> bool {
        regexp_test(self.0, text(input))
    }
}

/// An opened `Cache`.
#[derive(Clone, Copy)]
pub struct Cache(JsValue);

impl Cache {
    pub async fn open(name: &str) -> Result<Self, Error> {
        settle(storage_open(caches(), text(name))).await.map(Cache)
    }

    pub async fn match_request(self, request: Request) -> Result<Option<Response>, Error> {
        settle(cache_match(self.0, request.0)).await.map(response)
    }

    pub async fn match_url(self, url: &str) -> Result<Option<Response>, Error> {
        settle(cache_match(self.0, text(url))).await.map(response)
    }

    pub async fn put(self, request: Request, response: Response) -> Result<(), Error> {
        settle(cache_put(self.0, request.0, response.0)).await.map(|_| ())
    }

    pub async fn delete(self, request: Request) -> Result<bool, Error> {
        settle(cache_delete(self.0, request.0)).await.map(|deleted| deleted.is_truthy())
    }

    pub async fn keys(self) -> Result<Vec<Request>, Error> {
        settle(cache_keys(self.0)).await.map(requests)
    }

    /// `cache.keys(request, {ignoreSearch: true})`.
    pub async fn keys_ignoring_search(self, request: Request) -> Result<Vec<Request>, Error> {
        let options = object();
        set(options, "ignoreSearch", JsValue::erase(true));
        settle(cache_keys_matching(self.0, request.0, options)).await.map(requests)
    }
}

fn response(value: JsValue) -> Option<Response> {
    if value.is_undefined() { None } else { Some(Response(value)) }
}

fn requests(list: JsValue) -> Vec<Request> {
    (0..length(list)).map(|i| Request(at(list, i))).collect()
}

/// `caches.match(request, {cacheName})`, which opens no cache.
pub async fn caches_match(request: Request, cache_name: &str) -> Result<Option<Response>, Error> {
    let options = object();
    set(options, "cacheName", text(cache_name));
    settle(storage_match(caches(), request.0, options)).await.map(response)
}

pub async fn caches_keys() -> Result<Vec<String>, Error> {
    let names = settle(storage_keys(caches())).await?;
    Ok((0..length(names)).map(|i| string(at(names, i))).collect())
}

pub async fn caches_delete(name: &str) -> Result<bool, Error> {
    settle(storage_delete(caches(), text(name))).await.map(|deleted| deleted.is_truthy())
}

/// The global `fetch(request)`.
pub async fn fetch(request: Request) -> Result<Response, Error> {
    settle(host_fetch(request.0)).await.map(Response)
}

/// `event.waitUntil(promise)` on a host ExtendableEvent.
pub fn wait_until(event: JsValue, promise: JsValue) {
    event_wait_until(event, promise);
}
