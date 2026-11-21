//! The adapter (CONTRACT.md "Steps"): runs one script of Workbox API calls against this port,
//! with the contract's fixture plugins and event stand-ins, and builds the canonical output. The
//! scenario's caches are deleted on every path once it has reported, or failed.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::Future;
use mires_js::collections::HashMap;
use mires_js::{JsValue, await_promise, callback0, future_to_promise, invoke};

use crate::host::{self, Cache, RegExp, Request, RequestInit, Response, Url, array, at, define_value, get, length, object, push, set, string, text};
use crate::precaching::{precache_route, ManifestEntry, PrecacheController, PrecacheFallbackPlugin, PrecacheRouteOptions, RequestInfo};
use crate::routing::{Handler, MatchContext, Matcher, Route, Router};
use crate::strategies::{plugin, Callback, Cx, Hook, Kind, Mode, Plugin, PluginRef, Strategy};
use crate::workbox_core::{event, Error, Event, ExtendableEvent, Params, Task};

const FIXTURE: &str = "experiments/library-matrix/libraries/workbox/";
/// Well-formed SRI metadata that matches no fixture, so the fetch fails integrity.
const WRONG_INTEGRITY: &str = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

pub fn run(input: JsValue) -> JsValue {
    future_to_promise(async move { Scenario::new(input).run().await.map_err(Error::value) })
}

/// What the fixture plugins share with the scenario: the location base, the cache prefix, the
/// network fixture's source and the handler traces.
struct Fixture {
    base: String,
    prefix: String,
    source: RefCell<String>,
    handlers: JsValue,
}

impl Fixture {
    /// A URL under the page directory prints relative; any other URL prints whole.
    fn relative<'a>(&self, url: &'a str) -> &'a str {
        url.strip_prefix(self.base.as_str()).unwrap_or(url)
    }

    fn cache_name(&self, name: &str) -> String {
        let mut full = self.prefix.clone();
        full.push('-');
        full.push_str(name);
        full
    }

    /// The scenario's own name for one of its caches, or `None` for another cache.
    fn cache_label<'a>(&self, name: &'a str) -> Option<&'a str> {
        name.strip_prefix(self.prefix.as_str())?.strip_prefix('-')
    }

    fn label<'a>(&self, name: &'a str) -> &'a str {
        self.cache_label(name).unwrap_or(name)
    }

    /// A WorkboxError message without the location base and the cache prefix.
    fn scrub(&self, message: &str) -> String {
        let mut prefix = self.prefix.clone();
        prefix.push('-');
        message.replace(self.base.as_str(), "").replace(prefix.as_str(), "")
    }

    fn source_is(&self, source: &str) -> bool {
        *self.source.borrow() == source
    }
}

fn list(values: &[JsValue]) -> JsValue {
    let out = array();
    for value in values {
        push(out, *value);
    }
    out
}

fn number(value: usize) -> JsValue {
    JsValue::erase(value as f64)
}

fn status(response: Option<Response>) -> JsValue {
    response.map_or(JsValue::null(), |response| JsValue::erase(f64::from(response.status())))
}

/// An Error's name; a thrown non-Error (the fail fixture's `value`) prints as its string.
fn name_of(error: Error) -> JsValue {
    if error.is_error() { get(error.value(), "name") } else { host::string_of(error.value()) }
}

fn optional_string(value: JsValue) -> Option<String> {
    value.is_string().then(|| string(value))
}

fn strings(values: impl IntoIterator<Item = JsValue>) -> Vec<String> {
    values.into_iter().map(string).collect()
}

fn items(list: JsValue) -> impl Iterator<Item = JsValue> {
    (0..length(list)).map(move |i| at(list, i))
}

/// A manifest entry from the corpus: falsy, a string, or a `PrecacheEntry` object.
fn manifest_entry(value: JsValue) -> ManifestEntry {
    if !value.is_truthy() {
        ManifestEntry::Falsy(value)
    } else if value.is_string() {
        ManifestEntry::Url(string(value))
    } else {
        ManifestEntry::Entry {
            url: optional_string(get(value, "url")),
            revision: get(value, "revision"),
            integrity: optional_string(get(value, "integrity")),
            source: value,
        }
    }
}

/// The stand-in ExtendableEvent of one event-driven step: it collects its lifetime promises.
struct StandInEvent {
    kind: &'static str,
    lifetimes: Rc<RefCell<Vec<JsValue>>>,
}

impl ExtendableEvent for StandInEvent {
    fn event_type(&self) -> String {
        String::from(self.kind)
    }
    fn wait_until(&self, promise: JsValue) {
        self.lifetimes.borrow_mut().push(promise);
    }
}

/// A native event the caller supplied (service-worker mode).
struct HostEvent(JsValue);

impl ExtendableEvent for HostEvent {
    fn event_type(&self) -> String {
        string(get(self.0, "type"))
    }
    fn wait_until(&self, promise: JsValue) {
        host::wait_until(self.0, promise);
    }
}

/// Marks a response, and every clone of it, as a followed redirect leaves it.
fn redirected(response: Response) -> Response {
    let value = response.value();
    let clone = get(value, "clone");
    define_value(value, "redirected", JsValue::erase(true));
    define_value(value, "clone", callback0(move || redirected(Response::from_value(invoke(clone, value, &[]))).value()));
    response
}

/// The network fixture: every fetch goes to one of the two fixture files, or fails.
struct Network(Rc<Fixture>);

impl Plugin for Network {
    fn defines(&self, callback: Callback) -> bool {
        matches!(callback, Callback::RequestWillFetch | Callback::FetchDidSucceed)
    }
    fn request_will_fetch<'a>(&'a self, _: Cx<'a>, request: Request) -> Hook<'a, Request> {
        Box::pin(async move {
            if self.0.source_is("offline") {
                return Err(Error::Host(host::type_error("offline")));
            }
            let mut path = String::from(FIXTURE);
            path.push_str(if self.0.source_is("beta") { "beta.mjs" } else { "alpha.mjs" });
            let target = Url::parse(&path, &host::location_href())?.href();
            let integrity = if self.0.source_is("broken") { String::from(WRONG_INTEGRITY) } else { request.integrity() };
            Request::new(&target, &RequestInit { integrity: Some(&integrity), ..RequestInit::default() })
        })
    }
    fn fetch_did_succeed<'a>(&'a self, _: Cx<'a>, _: Request, response: Response) -> Hook<'a, Response> {
        Box::pin(async move {
            if self.0.source_is("missing") {
                return Response::new(text("missing"), JsValue::erase(404.0));
            }
            Ok(if self.0.source_is("redirected") { redirected(response) } else { response })
        })
    }
}

/// The trace plugin: every lifecycle callback but cacheWillUpdate, recorded per handler.
struct Trace {
    label: String,
    fixture: Rc<Fixture>,
}

impl Trace {
    /// Appends to the log this plugin's handlerWillStart created; without one (a plugin before
    /// it failed that callback) the record is dropped.
    fn record(&self, cx: Cx<'_>, record: &[JsValue]) {
        let log = cx.state.map(|state| state.get("log")).filter(|log| !log.is_undefined());
        if let Some(log) = log {
            push(log, list(record));
        }
    }
    fn url(&self, request: Request) -> JsValue {
        text(self.fixture.relative(&request.url()))
    }
}

impl Plugin for Trace {
    fn defines(&self, callback: Callback) -> bool {
        callback != Callback::CacheWillUpdate
    }
    fn handler_will_start<'a>(&'a self, cx: Cx<'a>, request: Request) -> Hook<'a, ()> {
        Box::pin(async move {
            let log = array();
            if let Some(state) = cx.state {
                state.set("log", log);
            }
            push(self.fixture.handlers, list(&[text(&self.label), self.url(request), log]));
            Ok(())
        })
    }
    fn cache_key_will_be_used<'a>(&'a self, cx: Cx<'a>, mode: Mode, request: Request, _: Option<&'a Params>) -> Hook<'a, Request> {
        Box::pin(async move {
            self.record(cx, &[text("cacheKeyWillBeUsed"), text(mode.name()), self.url(request)]);
            Ok(request)
        })
    }
    fn cached_response_will_be_used<'a>(&'a self, cx: Cx<'a>, cache_name: &'a str, request: Request, cached: Option<Response>) -> Hook<'a, Option<Response>> {
        Box::pin(async move {
            self.record(cx, &[text("cachedResponseWillBeUsed"), text(self.fixture.label(cache_name)), self.url(request), status(cached)]);
            Ok(cached)
        })
    }
    fn request_will_fetch<'a>(&'a self, cx: Cx<'a>, request: Request) -> Hook<'a, Request> {
        Box::pin(async move {
            self.record(cx, &[text("requestWillFetch"), self.url(request)]);
            Ok(request)
        })
    }
    fn fetch_did_succeed<'a>(&'a self, cx: Cx<'a>, request: Request, response: Response) -> Hook<'a, Response> {
        Box::pin(async move {
            self.record(cx, &[text("fetchDidSucceed"), self.url(request), status(Some(response))]);
            Ok(response)
        })
    }
    fn fetch_did_fail<'a>(&'a self, cx: Cx<'a>, error: Error, original: Request, request: Request) -> Hook<'a, ()> {
        Box::pin(async move {
            self.record(cx, &[text("fetchDidFail"), self.url(original), self.url(request), name_of(error)]);
            Ok(())
        })
    }
    fn cache_did_update<'a>(&'a self, cx: Cx<'a>, cache_name: &'a str, request: Request, old: Option<Response>, new: Response) -> Hook<'a, ()> {
        Box::pin(async move {
            self.record(cx, &[text("cacheDidUpdate"), text(self.fixture.label(cache_name)), self.url(request), status(old), status(Some(new))]);
            Ok(())
        })
    }
    fn handler_will_respond<'a>(&'a self, cx: Cx<'a>, _: Request, response: Response) -> Hook<'a, Response> {
        Box::pin(async move {
            self.record(cx, &[text("handlerWillRespond"), status(Some(response))]);
            Ok(response)
        })
    }
    fn handler_did_respond<'a>(&'a self, cx: Cx<'a>, _: Request, response: Option<Response>) -> Hook<'a, ()> {
        Box::pin(async move {
            self.record(cx, &[text("handlerDidRespond"), status(response)]);
            Ok(())
        })
    }
    fn handler_did_complete<'a>(&'a self, cx: Cx<'a>, _: Request, response: Option<Response>, error: Option<Error>) -> Hook<'a, ()> {
        Box::pin(async move {
            let error = error.map_or(JsValue::null(), name_of);
            self.record(cx, &[text("handlerDidComplete"), status(response), error]);
            Ok(())
        })
    }
    fn handler_did_error<'a>(&'a self, cx: Cx<'a>, _: Request, error: Error) -> Hook<'a, Option<Response>> {
        Box::pin(async move {
            self.record(cx, &[text("handlerDidError"), name_of(error)]);
            Ok(None)
        })
    }
}

/// A cacheWillUpdate that accepts every response (`keep`) or none (`drop`).
struct Cacheability {
    keep: bool,
}

impl Plugin for Cacheability {
    fn defines(&self, callback: Callback) -> bool {
        callback == Callback::CacheWillUpdate
    }
    fn cache_will_update<'a>(&'a self, _: Cx<'a>, _: Request, response: Response) -> Hook<'a, Option<Response>> {
        Box::pin(async move { Ok(self.keep.then_some(response)) })
    }
}

/// A plugin whose one callback throws `new Error('fail')`, or, with `value`, that value itself
/// (a non-Error).
struct Fail {
    callback: Callback,
    value: Option<JsValue>,
}

impl Fail {
    fn fail<'a, T: 'a>(&'a self) -> Hook<'a, T> {
        let thrown = self.value.unwrap_or_else(|| host::error("fail"));
        Box::pin(async move { Err(Error::Host(thrown)) })
    }
}

impl Plugin for Fail {
    fn defines(&self, callback: Callback) -> bool {
        callback == self.callback
    }
    fn handler_will_start<'a>(&'a self, _: Cx<'a>, _: Request) -> Hook<'a, ()> {
        self.fail()
    }
    fn cache_key_will_be_used<'a>(&'a self, _: Cx<'a>, _: Mode, _: Request, _: Option<&'a Params>) -> Hook<'a, Request> {
        self.fail()
    }
    fn cached_response_will_be_used<'a>(&'a self, _: Cx<'a>, _: &'a str, _: Request, _: Option<Response>) -> Hook<'a, Option<Response>> {
        self.fail()
    }
    fn cache_will_update<'a>(&'a self, _: Cx<'a>, _: Request, _: Response) -> Hook<'a, Option<Response>> {
        self.fail()
    }
    fn cache_did_update<'a>(&'a self, _: Cx<'a>, _: &'a str, _: Request, _: Option<Response>, _: Response) -> Hook<'a, ()> {
        self.fail()
    }
    fn request_will_fetch<'a>(&'a self, _: Cx<'a>, _: Request) -> Hook<'a, Request> {
        self.fail()
    }
    fn fetch_did_succeed<'a>(&'a self, _: Cx<'a>, _: Request, _: Response) -> Hook<'a, Response> {
        self.fail()
    }
    fn fetch_did_fail<'a>(&'a self, _: Cx<'a>, _: Error, _: Request, _: Request) -> Hook<'a, ()> {
        self.fail()
    }
    fn handler_will_respond<'a>(&'a self, _: Cx<'a>, _: Request, _: Response) -> Hook<'a, Response> {
        self.fail()
    }
    fn handler_did_respond<'a>(&'a self, _: Cx<'a>, _: Request, _: Option<Response>) -> Hook<'a, ()> {
        self.fail()
    }
    fn handler_did_complete<'a>(&'a self, _: Cx<'a>, _: Request, _: Option<Response>, _: Option<Error>) -> Hook<'a, ()> {
        self.fail()
    }
    fn handler_did_error<'a>(&'a self, _: Cx<'a>, _: Request, _: Error) -> Hook<'a, Option<Response>> {
        self.fail()
    }
}

fn callback_named(name: &str) -> Callback {
    match name {
        "handlerWillStart" => Callback::HandlerWillStart,
        "cacheKeyWillBeUsed" => Callback::CacheKeyWillBeUsed,
        "cachedResponseWillBeUsed" => Callback::CachedResponseWillBeUsed,
        "cacheWillUpdate" => Callback::CacheWillUpdate,
        "cacheDidUpdate" => Callback::CacheDidUpdate,
        "requestWillFetch" => Callback::RequestWillFetch,
        "fetchDidSucceed" => Callback::FetchDidSucceed,
        "fetchDidFail" => Callback::FetchDidFail,
        "handlerWillRespond" => Callback::HandlerWillRespond,
        "handlerDidRespond" => Callback::HandlerDidRespond,
        "handlerDidComplete" => Callback::HandlerDidComplete,
        "handlerDidError" => Callback::HandlerDidError,
        _ => panic!("not a plugin callback"),
    }
}

struct Scenario {
    input: JsValue,
    fixture: Rc<Fixture>,
    owned: bool,
    controllers: RefCell<HashMap<String, Rc<PrecacheController>>>,
    routers: RefCell<HashMap<String, Rc<Router>>>,
    routes: RefCell<HashMap<String, Rc<Route>>>,
    results: JsValue,
    events: JsValue,
}

impl Scenario {
    fn new(input: JsValue) -> Self {
        let given = get(input, "cachePrefix");
        let owned = given.is_undefined() || given.is_null();
        let prefix = if owned {
            let mut prefix = String::from("mires-workbox-");
            prefix.push_str(&host::random_uuid());
            prefix
        } else {
            string(given)
        };
        let base = Url::parse("./", &host::location_href()).map(|url| url.href()).unwrap_or_default();
        Scenario {
            input,
            fixture: Rc::new(Fixture { base, prefix, source: RefCell::new(String::from("alpha")), handlers: array() }),
            owned,
            controllers: RefCell::new(HashMap::new()),
            routers: RefCell::new(HashMap::new()),
            routes: RefCell::new(HashMap::new()),
            results: array(),
            events: array(),
        }
    }

    /// Runs every step, reports, and deletes the scenario's caches whatever happened.
    async fn run(self) -> Result<JsValue, Error> {
        let outcome = self.report().await;
        if self.owned {
            self.delete_caches().await?;
        }
        outcome
    }

    async fn report(&self) -> Result<JsValue, Error> {
        for step in items(get(self.input, "steps")) {
            let result = match self.step(step).await {
                Ok(result) => result,
                Err(error) => self.failure(error),
            };
            push(self.results, result);
        }
        let out = object();
        set(out, "results", self.results);
        set(out, "events", self.events);
        set(out, "handlers", self.fixture.handlers);
        set(out, "caches", self.caches().await?);
        Ok(out)
    }

    async fn delete_caches(&self) -> Result<(), Error> {
        for name in host::caches_keys().await? {
            if self.fixture.cache_label(&name).is_some() {
                host::caches_delete(&name).await?;
            }
        }
        Ok(())
    }

    fn controller(&self, step: JsValue, key: &str) -> Rc<PrecacheController> {
        let id = string(get(step, key));
        Rc::clone(self.controllers.borrow().get(id.as_str()).expect("a controller the script created"))
    }

    fn router(&self, step: JsValue) -> Rc<Router> {
        let id = string(get(step, "router"));
        Rc::clone(self.routers.borrow().get(id.as_str()).expect("a router the script created"))
    }

    fn route(&self, step: JsValue) -> Rc<Route> {
        let id = string(get(step, "route"));
        Rc::clone(self.routes.borrow().get(id.as_str()).expect("a route the script registered"))
    }

    async fn step(&self, step: JsValue) -> Result<JsValue, Error> {
        let op = string(get(step, "op"));
        match op.as_str() {
            "network" => {
                *self.fixture.source.borrow_mut() = string(get(step, "source"));
                Ok(JsValue::null())
            }
            "controller" => {
                let cache_name = self.fixture.cache_name(&string(get(step, "cacheName")));
                let fallback_to_network = !get(step, "fallbackToNetwork").strictly_equals(JsValue::erase(false));
                let controller = PrecacheController::new(cache_name, self.plugins(get(step, "plugins")), fallback_to_network);
                self.controllers.borrow_mut().insert(string(get(step, "id")), controller);
                Ok(JsValue::null())
            }
            "addToCacheList" => {
                let entries: Vec<ManifestEntry> = items(get(step, "entries")).map(manifest_entry).collect();
                self.controller(step, "controller").add_to_cache_list(&entries)?;
                Ok(JsValue::null())
            }
            "addPlugins" => {
                let controller = self.controller(step, "controller");
                let added: Vec<PluginRef> = items(get(step, "plugins")).map(|spec| self.plugin(spec)).collect();
                controller.strategy().plugins.borrow_mut().extend(added);
                Ok(JsValue::null())
            }
            "install" => {
                let controller = self.controller(step, "controller");
                let result = self.lifetime("install", |event| {
                    let task = controller.install(event);
                    async move { task.get().await }
                }).await?;
                let out = object();
                set(out, "updatedURLs", self.urls(&result.updated_urls));
                set(out, "notUpdatedURLs", self.urls(&result.not_updated_urls));
                Ok(out)
            }
            "activate" => {
                let controller = self.controller(step, "controller");
                let result = self.lifetime("activate", |event| {
                    let task = controller.activate(event);
                    async move { task.get().await }
                }).await?;
                let out = object();
                set(out, "deletedURLs", self.urls(&result.deleted_urls));
                Ok(out)
            }
            "getCachedURLs" => {
                let urls: Vec<String> = self.controller(step, "controller").urls_to_cache_keys().into_iter().map(|(url, _)| url).collect();
                Ok(self.urls(&urls))
            }
            "getURLsToCacheKeys" => {
                let out = array();
                for (url, key) in self.controller(step, "controller").urls_to_cache_keys() {
                    push(out, list(&[self.url(&url), self.url(&key)]));
                }
                Ok(out)
            }
            "getCacheKeyForURL" => {
                let key = self.controller(step, "controller").cache_key_for_url(&string(get(step, "url")))?;
                Ok(key.map_or(JsValue::null(), |key| self.url(&key)))
            }
            "matchPrecache" => {
                let url = string(get(step, "url"));
                let request = if get(step, "request").is_truthy() { RequestInfo::Request(Request::new(&url, &RequestInit::default())?) } else { RequestInfo::Url(url) };
                match self.controller(step, "controller").match_precache(request).await? {
                    Some(response) => self.record(response).await,
                    None => Ok(JsValue::null()),
                }
            }
            "router" => {
                self.routers.borrow_mut().insert(string(get(step, "id")), Rc::new(Router::default()));
                Ok(JsValue::null())
            }
            "precacheRoute" => {
                let options = self.route_options(get(step, "options"))?;
                let route = precache_route(self.controller(step, "controller"), options);
                self.register(step, route)
            }
            "route" => {
                let fixture = Rc::clone(&self.fixture);
                let matcher: Matcher = if get(step, "crossOrigin").is_truthy() {
                    Box::new(|context: &MatchContext<'_>| Ok((!context.same_origin).then(Params::default)))
                } else {
                    let prefix = string(get(step, "prefix"));
                    Box::new(move |context: &MatchContext<'_>| Ok(fixture.relative(&context.url.href()).starts_with(prefix.as_str()).then(Params::default)))
                };
                let handler = self.handler(get(step, "handler"))?;
                self.register(step, Route::new(matcher, handler, self.method(step)))
            }
            "defaultHandler" => {
                let router = self.router(step);
                router.set_default_handler(self.handler(get(step, "handler"))?, self.method(step));
                Ok(JsValue::null())
            }
            "catchHandler" => {
                if get(step, "route").is_undefined() {
                    let router = self.router(step);
                    router.set_catch_handler(self.handler(get(step, "handler"))?);
                } else {
                    let route = self.route(step);
                    route.set_catch_handler(self.handler(get(step, "handler"))?);
                }
                Ok(JsValue::null())
            }
            "unregisterRoute" => {
                self.router(step).unregister_route(&self.route(step))?;
                Ok(JsValue::null())
            }
            "fetch" => {
                let url = get(step, "url");
                let request = if url.is_undefined() {
                    Request::from_value(get(self.input, "request"))
                } else {
                    let (method, mode, integrity) = (optional_string(get(step, "method")), optional_string(get(step, "mode")), optional_string(get(step, "integrity")));
                    let init = RequestInit { method: method.as_deref(), mode: mode.as_deref(), integrity: integrity.as_deref(), ..RequestInit::default() };
                    Request::new(&string(url), &init)?
                };
                let router = self.router(step);
                let response = self.lifetime("fetch", |event| {
                    let task = router.handle_request(request, event);
                    async move {
                        match task? {
                            Some(task) => task.get().await.map(Some),
                            None => Ok(None),
                        }
                    }
                }).await?;
                match response {
                    Some(response) => self.record(response).await,
                    None => Ok(JsValue::null()),
                }
            }
            "caches" => self.caches().await,
            _ => panic!("not a step"),
        }
    }

    fn method(&self, step: JsValue) -> String {
        optional_string(get(step, "method")).unwrap_or_else(|| String::from("GET"))
    }

    fn register(&self, step: JsValue, route: Route) -> Result<JsValue, Error> {
        let route = Rc::new(route);
        if let Some(id) = optional_string(get(step, "id")) {
            self.routes.borrow_mut().insert(id, Rc::clone(&route));
        }
        self.router(step).register_route(route);
        Ok(JsValue::null())
    }

    /// The network fixture, then the plugins the script names.
    fn plugins(&self, specs: JsValue) -> Vec<PluginRef> {
        let mut plugins = Vec::from([plugin(Network(Rc::clone(&self.fixture)))]);
        if !specs.is_undefined() {
            plugins.extend(items(specs).map(|spec| self.plugin(spec)));
        }
        plugins
    }

    fn plugin(&self, spec: JsValue) -> PluginRef {
        match string(get(spec, "kind")).as_str() {
            "trace" => plugin(Trace { label: string(get(spec, "label")), fixture: Rc::clone(&self.fixture) }),
            "keep" => plugin(Cacheability { keep: true }),
            "drop" => plugin(Cacheability { keep: false }),
            "fail" => {
                let value = get(spec, "value");
                plugin(Fail { callback: callback_named(&string(get(spec, "on"))), value: (!value.is_undefined()).then_some(value) })
            }
            "fallback" => plugin(PrecacheFallbackPlugin::new(string(get(spec, "url")), self.controller(spec, "controller"))),
            _ => panic!("not a fixture plugin"),
        }
    }

    /// A route handler: a strategy, a controller's precache strategy, a handler bound to a
    /// precached URL, or the `respond` callback (an async function with a fixed response).
    fn handler(&self, spec: JsValue) -> Result<Rc<Handler>, Error> {
        if let Some(name) = optional_string(get(spec, "strategy")) {
            let kind = match name.as_str() {
                "CacheFirst" => Kind::CacheFirst,
                "CacheOnly" => Kind::CacheOnly,
                "NetworkFirst" => Kind::NetworkFirst,
                "NetworkOnly" => Kind::NetworkOnly,
                _ => panic!("not a strategy"),
            };
            let cache_name = self.fixture.cache_name(&string(get(spec, "cacheName")));
            return Ok(Rc::new(Handler::Strategy(Strategy::new(kind, cache_name, self.plugins(get(spec, "plugins"))))));
        }
        if !get(spec, "precache").is_undefined() {
            return Ok(Rc::new(Handler::Strategy(Rc::clone(self.controller(spec, "precache").strategy()))));
        }
        if let Some(url) = optional_string(get(spec, "bound")) {
            return Ok(Rc::new(self.controller(spec, "controller").create_handler_bound_to_url(&url)?));
        }
        let (body, status) = (get(spec, "respond"), get(spec, "status"));
        Ok(Rc::new(Handler::Callback(Box::new(move |_| Task::spawn(async move { Response::new(body, status) })))))
    }

    fn route_options(&self, options: JsValue) -> Result<PrecacheRouteOptions, Error> {
        let mut route = PrecacheRouteOptions::defaults()?;
        if options.is_undefined() {
            return Ok(route);
        }
        let directory_index = get(options, "directoryIndex");
        if !directory_index.is_undefined() {
            route.directory_index = directory_index.is_truthy().then(|| string(directory_index));
        }
        let clean_urls = get(options, "cleanURLs");
        if !clean_urls.is_undefined() {
            route.clean_urls = clean_urls.is_truthy();
        }
        let ignore = get(options, "ignoreURLParametersMatching");
        if !ignore.is_undefined() {
            route.ignore_url_parameters_matching = items(ignore).map(|source| RegExp::new(&string(source))).collect::<Result<_, _>>()?;
        }
        let manipulation = get(options, "urlManipulation");
        if !manipulation.is_undefined() {
            let suffixes = strings(items(manipulation));
            route.url_manipulation = Some(Box::new(move |url: &Url| {
                let mut variants = Vec::new();
                for suffix in &suffixes {
                    let variant = Url::absolute(&url.href())?;
                    variant.set_pathname(&(variant.pathname() + suffix));
                    variants.push(variant);
                }
                Ok(variants)
            }));
        }
        Ok(route)
    }

    /// Runs one event-driven step under a fresh stand-in event and waits for every lifetime it
    /// was given, or, in service-worker mode, under the caller's native event.
    async fn lifetime<T, F, Fut>(&self, kind: &'static str, action: F) -> Result<T, Error>
    where
        F: FnOnce(Event) -> Fut,
        Fut: Future<Output = Result<T, Error>>,
    {
        let native = get(self.input, "event");
        if !native.is_undefined() {
            return action(event(HostEvent(native))).await;
        }
        let lifetimes = Rc::new(RefCell::new(Vec::new()));
        let outcome = action(event(StandInEvent { kind, lifetimes: Rc::clone(&lifetimes) })).await;
        let mut count = 0;
        loop {
            let batch = core::mem::take(&mut *lifetimes.borrow_mut());
            if batch.is_empty() {
                break;
            }
            count += batch.len();
            for promise in batch {
                let _ = await_promise(promise).await;
            }
        }
        push(self.events, list(&[text(kind), number(count)]));
        outcome
    }

    fn url(&self, url: &str) -> JsValue {
        text(self.fixture.relative(url))
    }

    fn urls(&self, urls: &[String]) -> JsValue {
        let out = array();
        for url in urls {
            push(out, self.url(url));
        }
        out
    }

    /// `{status, url, body}` of a response, reading its body.
    async fn record(&self, response: Response) -> Result<JsValue, Error> {
        let status = status(Some(response));
        let url = self.url(&response.url());
        let body = response.text().await?;
        let out = object();
        set(out, "status", status);
        set(out, "url", url);
        set(out, "body", body);
        Ok(out)
    }

    /// `{error, message}` for a WorkboxError, `{error}` for another Error, `{thrown}` for a
    /// thrown non-Error.
    fn failure(&self, error: Error) -> JsValue {
        let out = object();
        match error {
            Error::Workbox(_) => {
                set(out, "error", get(error.value(), "name"));
                set(out, "message", text(&self.fixture.scrub(&string(error.message()))));
            }
            Error::Host(_) if error.is_error() => set(out, "error", get(error.value(), "name")),
            Error::Host(value) => set(out, "thrown", host::string_of(value)),
        }
        out
    }

    /// Every cache of the scenario, in CacheStorage order, with each entry's response record.
    async fn caches(&self) -> Result<JsValue, Error> {
        let out = array();
        for name in host::caches_keys().await? {
            let Some(label) = self.fixture.cache_label(&name) else { continue };
            let cache = Cache::open(&name).await?;
            let entries = array();
            for request in cache.keys().await? {
                let response = cache.match_request(request).await?.ok_or_else(|| Error::Host(host::type_error("an entry without a response")))?;
                push(entries, list(&[self.url(&request.url()), self.record(response).await?]));
            }
            push(out, list(&[text(label), entries]));
        }
        Ok(out)
    }
}
