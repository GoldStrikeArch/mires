//! workbox-core: `WorkboxError` and its production messages, `copyResponse`,
//! `cacheMatchIgnoreParams`, `waitUntil`, the ExtendableEvent a strategy runs under, and the
//! eager, shareable task that stands in for a JavaScript promise.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;
use core::future::Future;
use mires_js::{JsValue, await_promise, future_to_promise};

use crate::host::{self, Cache, Request, Response, Url, array, get, object, push, set, text};

/// A thrown value: a `WorkboxError` this port created, or whatever a host API or a plugin threw.
/// Both carry the JavaScript value, so a plugin, a waiting event and an error's `details` see the
/// object JavaScript would.
#[derive(Clone, Copy)]
pub enum Error {
    Workbox(JsValue),
    Host(JsValue),
}

impl Error {
    /// `new WorkboxError(code, details)` from the production build: an `Error` whose own `name`
    /// is the code and own `details` the details, with the message `code :: [details]` (the
    /// production `messageGenerator` stringifies the details argument list, so an absent
    /// details object prints `[null]`).
    pub fn workbox(code: &str, details: Option<Details>) -> Self {
        let details = details.map_or(JsValue::undefined(), |details| details.0);
        let arguments = array();
        push(arguments, details);
        let mut message = String::from(code);
        message.push_str(" :: ");
        message.push_str(&host::stringify(arguments));
        let error = host::error(&message);
        set(error, "name", text(code));
        set(error, "details", details);
        Error::Workbox(error)
    }

    pub fn value(self) -> JsValue {
        match self {
            Error::Workbox(value) | Error::Host(value) => value,
        }
    }

    pub fn message(self) -> JsValue {
        get(self.value(), "message")
    }

    /// `error instanceof Error`: Workbox tests it before it keeps or reports a caught value,
    /// because a plugin may throw anything.
    pub fn is_error(self) -> bool {
        self.value().is_error()
    }
}

/// The details object of a `WorkboxError`, in insertion order.
pub struct Details(JsValue);

impl Details {
    pub fn new() -> Self {
        Details(object())
    }

    pub fn text(self, key: &str, value: &str) -> Self {
        set(self.0, key, text(value));
        self
    }

    pub fn number(self, key: &str, value: u16) -> Self {
        set(self.0, key, JsValue::erase(f64::from(value)));
        self
    }

    pub fn value(self, key: &str, value: JsValue) -> Self {
        set(self.0, key, value);
        self
    }
}

/// The event a strategy, `install` or `activate` runs under: a native ExtendableEvent, or the
/// stand-in a page supplies.
pub trait ExtendableEvent {
    fn event_type(&self) -> String;
    fn wait_until(&self, promise: JsValue);
}

/// A shared event. (mires lays out `Rc<Box<dyn _>>` but not `Rc<dyn _>`.)
pub type Event = Rc<Box<dyn ExtendableEvent>>;

pub fn event(event: impl ExtendableEvent + 'static) -> Event {
    Rc::new(Box::new(event))
}

/// A computation that starts at once and can be awaited any number of times, as a JavaScript
/// promise can: `spawn` runs the future up to its first suspension before it returns, and its
/// `promise` settles as the result does, for `event.waitUntil`.
pub struct Task<T> {
    promise: JsValue,
    result: Rc<RefCell<Option<Result<T, Error>>>>,
}

impl<T> Clone for Task<T> {
    fn clone(&self) -> Self {
        Task { promise: self.promise, result: Rc::clone(&self.result) }
    }
}

impl<T: Clone + 'static> Task<T> {
    pub fn spawn<F: Future<Output = Result<T, Error>> + 'static>(future: F) -> Self {
        let result = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&result);
        let promise = future_to_promise(async move {
            let outcome = future.await;
            let settled = match &outcome {
                Ok(_) => Ok(JsValue::undefined()),
                Err(error) => Err(error.value()),
            };
            *slot.borrow_mut() = Some(outcome);
            settled
        });
        Task { promise, result }
    }

    pub fn promise(&self) -> JsValue {
        self.promise
    }

    pub async fn get(&self) -> Result<T, Error> {
        let _ = await_promise(self.promise).await;
        self.result.borrow().clone().expect("a settled task holds its result")
    }
}

/// workbox-core's `waitUntil(event, asyncFn)`: the started task extends the event's lifetime and
/// is returned.
pub fn wait_until<T: Clone + 'static>(event: &Event, task: Task<T>) -> Task<T> {
    event.wait_until(task.promise());
    task
}

/// `copyResponse(response)`: refuses a cross-origin (or URL-less) response, then builds a new
/// Response from a clone's headers, status, status text and body. Workbox's
/// `canConstructResponseFromBodyStream` always answers false, so the body is read as a Blob.
pub async fn copy_response(response: Response) -> Result<Response, Error> {
    let url = response.url();
    let origin = if url.is_empty() { None } else { Some(Url::absolute(&url)?.origin()) };
    if origin.as_deref() != Some(host::location_origin().as_str()) {
        let origin = origin.as_deref().map_or(JsValue::null(), text);
        return Err(Error::workbox("cross-origin-copy-response", Some(Details::new().value("origin", origin))));
    }
    let cloned = response.duplicate();
    let body = cloned.blob().await?;
    Response::with_parts(body, cloned.headers(), cloned.status(), cloned.status_text())
}

fn strip_params(url: &str, ignore: &[&str]) -> Result<String, Error> {
    let stripped = Url::absolute(url)?;
    for param in ignore {
        stripped.delete_search_param(param);
    }
    Ok(stripped.href())
}

/// `cacheMatchIgnoreParams(cache, request, ignoreParams)`: an ordinary match when the request
/// has none of the parameters, else the first key that equals the request once both are
/// stripped.
pub async fn cache_match_ignore_params(cache: Cache, request: Request, ignore: &[&str]) -> Result<Option<Response>, Error> {
    let url = request.url();
    let stripped = strip_params(&url, ignore)?;
    if url == stripped {
        return cache.match_request(request).await;
    }
    for key in cache.keys_ignoring_search(request).await? {
        if strip_params(&key.url(), ignore)? == stripped {
            return cache.match_request(key).await;
        }
    }
    Ok(None)
}

/// The route parameters a handler receives. In this subset they come from a `PrecacheRoute`
/// match (`{cacheKey, integrity}`) or from `createHandlerBoundToURL` (`{cacheKey}`).
#[derive(Clone, Default)]
pub struct Params {
    pub cache_key: Option<String>,
    pub integrity: Option<String>,
}

impl Params {
    /// The `{}` that `Router.findMatchingRoute` replaces with `undefined`.
    pub fn is_empty(&self) -> bool {
        self.cache_key.is_none() && self.integrity.is_none()
    }
}

/// `HandlerCallbackOptions`: what a route handler is called with.
#[derive(Clone)]
pub struct HandlerOptions {
    pub request: Request,
    pub event: Event,
    pub params: Option<Params>,
}
