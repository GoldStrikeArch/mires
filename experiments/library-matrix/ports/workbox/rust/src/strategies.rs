//! workbox-strategies: the plugin interface, `Strategy` with its `handle`/`handleAll` lifecycle,
//! `StrategyHandler`, and the CacheFirst, CacheOnly, NetworkFirst and NetworkOnly strategies.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use mires_js::JsValue;
use mires_js::collections::HashMap;

use crate::host::{self, Cache, Deferred, Request, Response};
use crate::precaching::PrecacheStrategy;
use crate::workbox_core::{cache_match_ignore_params, Details, Error, Event, ExtendableEvent, HandlerOptions, Params, Task};

/// What a plugin callback returns: a future borrowing the plugin and its arguments.
pub type Hook<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + 'a>>;

/// The plugin callbacks Workbox runs, by name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Callback {
    HandlerWillStart,
    CacheKeyWillBeUsed,
    CachedResponseWillBeUsed,
    CacheWillUpdate,
    CacheDidUpdate,
    RequestWillFetch,
    FetchDidSucceed,
    FetchDidFail,
    HandlerWillRespond,
    HandlerDidRespond,
    HandlerDidComplete,
    HandlerDidError,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Read,
    Write,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Read => "read",
            Mode::Write => "write",
        }
    }
}

/// The state object a handler keeps for one plugin (`state` in every callback): a plain host
/// object, as Workbox's `MapLikeObject` is.
pub struct PluginState(JsValue);

impl PluginState {
    fn new() -> Self {
        PluginState(host::object())
    }

    pub fn get(&self, key: &str) -> JsValue {
        host::get(self.0, key)
    }

    pub fn set(&self, key: &str, value: JsValue) {
        host::set(self.0, key, value);
    }
}

/// What every callback receives besides its own arguments: the event and the plugin's state,
/// which is absent for a plugin added to the strategy after the handler was created.
#[derive(Clone, Copy)]
pub struct Cx<'a> {
    pub event: &'a dyn ExtendableEvent,
    pub state: Option<&'a PluginState>,
}

fn undefined_callback<'a, T: 'a>() -> Hook<'a, T> {
    Box::pin(async { unreachable!("Workbox calls only the callbacks a plugin defines") })
}

/// A plugin as a strategy holds it; its identity is the `Rc`. (mires lays out `Rc<Box<dyn _>>`
/// but not `Rc<dyn _>`.)
pub type PluginRef = Rc<Box<dyn Plugin>>;

pub fn plugin(plugin: impl Plugin + 'static) -> PluginRef {
    Rc::new(Box::new(plugin))
}

/// `WorkboxPlugin`: an object of optional lifecycle callbacks. `defines` answers `name in
/// plugin`; Workbox calls only the callbacks a plugin defines.
pub trait Plugin {
    fn defines(&self, callback: Callback) -> bool;

    fn handler_will_start<'a>(&'a self, _: Cx<'a>, _request: Request) -> Hook<'a, ()> {
        undefined_callback()
    }
    fn cache_key_will_be_used<'a>(&'a self, _: Cx<'a>, _mode: Mode, _request: Request, _params: Option<&'a Params>) -> Hook<'a, Request> {
        undefined_callback()
    }
    fn cached_response_will_be_used<'a>(&'a self, _: Cx<'a>, _cache_name: &'a str, _request: Request, _cached: Option<Response>) -> Hook<'a, Option<Response>> {
        undefined_callback()
    }
    fn cache_will_update<'a>(&'a self, _: Cx<'a>, _request: Request, _response: Response) -> Hook<'a, Option<Response>> {
        undefined_callback()
    }
    fn cache_did_update<'a>(&'a self, _: Cx<'a>, _cache_name: &'a str, _request: Request, _old: Option<Response>, _new: Response) -> Hook<'a, ()> {
        undefined_callback()
    }
    fn request_will_fetch<'a>(&'a self, _: Cx<'a>, _request: Request) -> Hook<'a, Request> {
        undefined_callback()
    }
    fn fetch_did_succeed<'a>(&'a self, _: Cx<'a>, _request: Request, _response: Response) -> Hook<'a, Response> {
        undefined_callback()
    }
    fn fetch_did_fail<'a>(&'a self, _: Cx<'a>, _error: Error, _original: Request, _request: Request) -> Hook<'a, ()> {
        undefined_callback()
    }
    fn handler_will_respond<'a>(&'a self, _: Cx<'a>, _request: Request, _response: Response) -> Hook<'a, Response> {
        undefined_callback()
    }
    fn handler_did_respond<'a>(&'a self, _: Cx<'a>, _request: Request, _response: Option<Response>) -> Hook<'a, ()> {
        undefined_callback()
    }
    fn handler_did_complete<'a>(&'a self, _: Cx<'a>, _request: Request, _response: Option<Response>, _error: Option<Error>) -> Hook<'a, ()> {
        undefined_callback()
    }
    fn handler_did_error<'a>(&'a self, _: Cx<'a>, _request: Request, _error: Error) -> Hook<'a, Option<Response>> {
        undefined_callback()
    }
}

/// The `_handle` of each Strategy subclass in the subset.
pub enum Kind {
    CacheFirst,
    CacheOnly,
    NetworkFirst,
    NetworkOnly,
    Precache(PrecacheStrategy),
}

/// `Strategy`: a cache name, a mutable plugin list and the subclass's `_handle`.
pub struct Strategy {
    pub cache_name: String,
    pub plugins: RefCell<Vec<PluginRef>>,
    pub kind: Kind,
}

impl Strategy {
    /// `new CacheFirst(options)` and its siblings. The cache name is always supplied
    /// (`cacheNames.getRuntimeName(cacheName)` returns it unchanged).
    pub fn new(kind: Kind, cache_name: String, plugins: Vec<PluginRef>) -> Rc<Self> {
        let mut plugins = plugins;
        // NetworkFirst puts its 200-or-opaque plugin first unless a plugin has a cacheWillUpdate.
        if matches!(kind, Kind::NetworkFirst) && !plugins.iter().any(|plugin| plugin.defines(Callback::CacheWillUpdate)) {
            plugins.insert(0, plugin(CacheOkAndOpaque));
        }
        Rc::new(Strategy { cache_name, plugins: RefCell::new(plugins), kind })
    }

    /// `handle(options)`: the response task of `handleAll`; the completion task keeps running.
    pub fn handle(self: &Rc<Self>, options: HandlerOptions) -> Task<Response> {
        self.handle_all(options).0
    }

    /// `handleAll(options)`: a handler for this request, its response task and its completion
    /// task, both already running.
    pub fn handle_all(self: &Rc<Self>, options: HandlerOptions) -> (Task<Response>, Task<()>) {
        let request = options.request;
        let handler = Rc::new(StrategyHandler::new(Rc::clone(self), options));
        let response_done = Task::spawn(get_response(Rc::clone(self), Rc::clone(&handler), request));
        let handler_done = Task::spawn(await_complete(response_done.clone(), handler, request));
        (response_done, handler_done)
    }

    async fn run(&self, request: Request, handler: &Rc<StrategyHandler>) -> Result<Response, Error> {
        match &self.kind {
            Kind::CacheFirst => cache_first(request, handler).await,
            Kind::CacheOnly => cache_only(request, handler).await,
            Kind::NetworkFirst => network_first(request, handler).await,
            Kind::NetworkOnly => network_only(request, handler).await,
            Kind::Precache(precache) => precache.handle(self, request, handler).await,
        }
    }
}



/// `_getResponse`: handlerWillStart, `_handle`, handlerDidError on failure, handlerWillRespond.
async fn get_response(strategy: Rc<Strategy>, handler: Rc<StrategyHandler>, request: Request) -> Result<Response, Error> {
    for (plugin, state) in handler.callbacks(Callback::HandlerWillStart) {
        plugin.handler_will_start(handler.cx(state), request).await?;
    }
    // As in Workbox, a response of type "error" stays the response unless a handlerDidError
    // callback replaces it, so it is returned rather than rethrown when none does.
    let mut response = None;
    let failure = match strategy.run(request, &handler).await {
        Ok(found) => {
            response = Some(found);
            (found.kind() == "error").then(|| Error::workbox("no-response", Some(Details::new().text("url", &request.url()))))
        }
        Err(error) => Some(error),
    };
    if let Some(error) = failure {
        if error.is_error() {
            for (plugin, state) in handler.callbacks(Callback::HandlerDidError) {
                response = plugin.handler_did_error(handler.cx(state), request, error).await?;
                if response.is_some() {
                    break;
                }
            }
        }
        if response.is_none() {
            return Err(error);
        }
    }
    let mut response = response.expect("a response or an error");
    for (plugin, state) in handler.callbacks(Callback::HandlerWillRespond) {
        response = plugin.handler_will_respond(handler.cx(state), request, response).await?;
    }
    Ok(response)
}

/// `_awaitComplete`: handlerDidRespond and the handler's extended lifetimes, then
/// handlerDidComplete with the first error among them, then the handler's own promise resolves.
async fn await_complete(response_done: Task<Response>, handler: Rc<StrategyHandler>, request: Request) -> Result<(), Error> {
    let response = response_done.get().await.ok();
    let finished = async {
        for (plugin, state) in handler.callbacks(Callback::HandlerDidRespond) {
            plugin.handler_did_respond(handler.cx(state), request, response).await?;
        }
        handler.done_waiting().await
    };
    let error = match finished.await {
        Err(error) if error.is_error() => Some(error),
        _ => None,
    };
    for (plugin, state) in handler.callbacks(Callback::HandlerDidComplete) {
        plugin.handler_did_complete(handler.cx(state), request, response, error).await?;
    }
    handler.destroy();
    error.map_or(Ok(()), Err)
}

/// `CacheFirst._handle`.
async fn cache_first(request: Request, handler: &Rc<StrategyHandler>) -> Result<Response, Error> {
    if let Some(response) = handler.cache_match(request).await? {
        return Ok(response);
    }
    match handler.fetch_and_cache_put(request).await {
        Ok(response) => Ok(response),
        Err(error) => Err(no_response(request, Some(error))),
    }
}

/// `CacheOnly._handle`.
async fn cache_only(request: Request, handler: &Rc<StrategyHandler>) -> Result<Response, Error> {
    match handler.cache_match(request).await? {
        Some(response) => Ok(response),
        None => Err(no_response(request, None)),
    }
}

/// `NetworkFirst._handle`, without `networkTimeoutSeconds`: the network promise raced alone,
/// then itself again, and both waits extend the handler's lifetime.
async fn network_first(request: Request, handler: &Rc<StrategyHandler>) -> Result<Response, Error> {
    let network = Task::spawn(network_first_promise(request, Rc::clone(handler)));
    // `Promise.race([network])` settles as `network` does.
    let raced = network.clone();
    handler.wait_until(lifetime_of(&raced));
    let response = Task::spawn(async move {
        match raced.get().await? {
            Some(response) => Ok(Some(response)),
            None => network.get().await,
        }
    });
    handler.wait_until(lifetime_of(&response));
    match response.get().await? {
        Some(response) => Ok(response),
        None => Err(no_response(request, None)),
    }
}

/// `_getNetworkPromise`: fetchAndCachePut, and the cache when that failed (or answered nothing).
async fn network_first_promise(request: Request, handler: Rc<StrategyHandler>) -> Result<Option<Response>, Error> {
    let (response, error) = match handler.fetch_and_cache_put(request).await {
        Ok(response) => (Some(response), None),
        Err(error) => (None, error.is_error().then_some(error)),
    };
    if error.is_none() && response.is_some() {
        return Ok(response);
    }
    handler.cache_match(request).await
}

/// A task's settling as a lifetime for `StrategyHandler::wait_until`.
fn lifetime_of<T: Clone + 'static>(task: &Task<T>) -> Task<()> {
    let task = task.clone();
    Task::spawn(async move { task.get().await.map(|_| ()) })
}

/// `cacheOkAndOpaquePlugin`: status 200, or 0 for an opaque response.
struct CacheOkAndOpaque;

impl Plugin for CacheOkAndOpaque {
    fn defines(&self, callback: Callback) -> bool {
        callback == Callback::CacheWillUpdate
    }
    fn cache_will_update<'a>(&'a self, _: Cx<'a>, _: Request, response: Response) -> Hook<'a, Option<Response>> {
        Box::pin(async move { Ok(matches!(response.status(), 200 | 0).then_some(response)) })
    }
}

/// `NetworkOnly._handle`, without `networkTimeoutSeconds`.
async fn network_only(request: Request, handler: &Rc<StrategyHandler>) -> Result<Response, Error> {
    match handler.fetch(request).await {
        Ok(response) => Ok(response),
        Err(error) => Err(no_response(request, Some(error))),
    }
}

/// `new WorkboxError('no-response', {url, error})`, where the caught error is kept only when it
/// is an `Error`.
fn no_response(request: Request, caught: Option<Error>) -> Error {
    let details = Details::new().text("url", &request.url());
    let details = match caught {
        Some(error) if error.is_error() => details.value("error", error.value()),
        _ => details,
    };
    Error::workbox("no-response", Some(details))
}

/// `StrategyHandler`: one request's run through a strategy.
pub struct StrategyHandler {
    pub request: Request,
    pub event: Event,
    pub params: Option<Params>,
    strategy: Rc<Strategy>,
    cache_keys: RefCell<HashMap<String, Request>>,
    extend_lifetime: RefCell<Vec<Task<()>>>,
    deferred: Deferred,
    states: Vec<(PluginRef, PluginState)>,
}

impl StrategyHandler {
    fn new(strategy: Rc<Strategy>, options: HandlerOptions) -> Self {
        // The state map covers the plugins present now; a plugin added later gets no state.
        let states = strategy.plugins.borrow().iter().map(|plugin| (Rc::clone(plugin), PluginState::new())).collect();
        let handler = StrategyHandler {
            request: options.request,
            event: options.event,
            params: options.params,
            strategy,
            cache_keys: RefCell::new(HashMap::new()),
            extend_lifetime: RefCell::new(Vec::new()),
            deferred: Deferred::new(),
            states,
        };
        handler.event.wait_until(handler.deferred.promise());
        handler
    }

    pub fn cx<'a>(&'a self, state: Option<&'a PluginState>) -> Cx<'a> {
        Cx { event: &**self.event, state }
    }

    /// `hasCallback(name)`, over the strategy's current plugins.
    pub fn has_callback(&self, callback: Callback) -> bool {
        self.strategy.plugins.borrow().iter().any(|plugin| plugin.defines(callback))
    }

    /// `iterateCallbacks(name)`: the strategy's plugins that define the callback, read live, so
    /// a plugin added while the handler runs is reached (without state).
    pub fn callbacks(&self, callback: Callback) -> Callbacks<'_> {
        Callbacks { handler: self, callback, index: 0 }
    }

    fn state_of(&self, plugin: &PluginRef) -> Option<&PluginState> {
        self.states.iter().find(|(known, _)| Rc::ptr_eq(known, plugin)).map(|(_, state)| state)
    }

    /// `fetch(input)`: requestWillFetch, the network, then fetchDidSucceed or fetchDidFail.
    pub async fn fetch(&self, input: Request) -> Result<Response, Error> {
        let mut request = input;
        let original = if self.has_callback(Callback::FetchDidFail) { Some(request.duplicate()) } else { None };
        for (plugin, state) in self.callbacks(Callback::RequestWillFetch) {
            match plugin.request_will_fetch(self.cx(state), request.duplicate()).await {
                Ok(next) => request = next,
                Err(error) if error.is_error() => {
                    let message = host::string(host::string_of(error.message()));
                    return Err(Error::workbox("plugin-error-request-will-fetch", Some(Details::new().text("thrownErrorMessage", &message))));
                }
                // A thrown non-Error ends the loop and the request goes out as it is.
                Err(_) => break,
            }
        }
        let filtered = request.duplicate();
        let attempt = async {
            let mut response = host::fetch(request).await?;
            for (plugin, state) in self.callbacks(Callback::FetchDidSucceed) {
                response = plugin.fetch_did_succeed(self.cx(state), filtered, response).await?;
            }
            Ok(response)
        };
        match attempt.await {
            Ok(response) => Ok(response),
            Err(error) => {
                if let Some(original) = original {
                    let (original, request) = (original.duplicate(), filtered.duplicate());
                    for (plugin, state) in self.callbacks(Callback::FetchDidFail) {
                        plugin.fetch_did_fail(self.cx(state), error, original, request).await?;
                    }
                }
                Err(error)
            }
        }
    }

    /// `fetchAndCachePut(input)`: the response now, and its cachePut as an extended lifetime.
    pub async fn fetch_and_cache_put(self: &Rc<Self>, input: Request) -> Result<Response, Error> {
        let response = self.fetch(input).await?;
        let copy = response.duplicate();
        let handler = Rc::clone(self);
        self.wait_until(Task::spawn(async move { handler.cache_put(input, copy).await.map(|_| ()) }));
        Ok(response)
    }

    /// `cacheMatch(key)`: `caches.match` of the read cache key in the strategy's cache, through
    /// cachedResponseWillBeUsed.
    pub async fn cache_match(&self, key: Request) -> Result<Option<Response>, Error> {
        let cache_name = self.strategy.cache_name.as_str();
        let effective = self.get_cache_key(key, Mode::Read).await?;
        let mut cached = host::caches_match(effective, cache_name).await?;
        for (plugin, state) in self.callbacks(Callback::CachedResponseWillBeUsed) {
            cached = plugin.cached_response_will_be_used(self.cx(state), cache_name, effective, cached).await?;
        }
        Ok(cached)
    }

    /// `cachePut(key, response)`: one timer task later, the write cache key, cacheWillUpdate
    /// (or the status-200 rule), the put, then cacheDidUpdate. Answers whether it cached.
    pub async fn cache_put(&self, key: Request, response: Response) -> Result<bool, Error> {
        host::timeout(0.0).await;
        let effective = self.get_cache_key(key, Mode::Write).await?;
        let Some(to_cache) = self.ensure_response_safe_to_cache(response).await? else {
            return Ok(false);
        };
        let cache_name = self.strategy.cache_name.as_str();
        let cache = Cache::open(cache_name).await?;
        let reports = self.has_callback(Callback::CacheDidUpdate);
        let old = if reports { cache_match_ignore_params(cache, effective.duplicate(), &["__WB_REVISION__"]).await? } else { None };
        if let Err(error) = cache.put(effective, if reports { to_cache.duplicate() } else { to_cache }).await {
            // Workbox rethrows an Error (after the registered quota callbacks, of which the
            // subset has none, for a QuotaExceededError) and goes on after anything else.
            if error.is_error() {
                return Err(error);
            }
        }
        for (plugin, state) in self.callbacks(Callback::CacheDidUpdate) {
            plugin.cache_did_update(self.cx(state), cache_name, effective, old, to_cache.duplicate()).await?;
        }
        Ok(true)
    }

    /// `getCacheKey(request, mode)`: cacheKeyWillBeUsed once per URL and mode for this handler.
    pub async fn get_cache_key(&self, request: Request, mode: Mode) -> Result<Request, Error> {
        let mut key = request.url();
        key.push_str(" | ");
        key.push_str(mode.name());
        if let Some(known) = self.cache_keys.borrow().get(key.as_str()) {
            return Ok(*known);
        }
        let mut effective = request;
        for (plugin, state) in self.callbacks(Callback::CacheKeyWillBeUsed) {
            effective = plugin.cache_key_will_be_used(self.cx(state), mode, effective, self.params.as_ref()).await?;
        }
        self.cache_keys.borrow_mut().insert(key, effective);
        Ok(effective)
    }

    /// `_ensureResponseSafeToCache(response)`: every cacheWillUpdate in turn until one refuses,
    /// or, without any, only a status 200.
    async fn ensure_response_safe_to_cache(&self, response: Response) -> Result<Option<Response>, Error> {
        let mut to_cache = response;
        let mut plugins_used = false;
        for (plugin, state) in self.callbacks(Callback::CacheWillUpdate) {
            plugins_used = true;
            match plugin.cache_will_update(self.cx(state), self.request, to_cache).await? {
                Some(next) => to_cache = next,
                None => return Ok(None),
            }
        }
        Ok(if plugins_used || to_cache.status() == 200 { Some(to_cache) } else { None })
    }

    /// `waitUntil(promise)`: extends the handler's lifetime until the task settles.
    pub fn wait_until(&self, task: Task<()>) {
        self.extend_lifetime.borrow_mut().push(task);
    }

    /// `doneWaiting()`: waits for every extended lifetime, including ones added meanwhile, and
    /// fails with the first rejection of a batch.
    pub async fn done_waiting(&self) -> Result<(), Error> {
        loop {
            let batch = core::mem::take(&mut *self.extend_lifetime.borrow_mut());
            if batch.is_empty() {
                return Ok(());
            }
            let mut first = None;
            for task in &batch {
                if let Err(error) = task.get().await {
                    first.get_or_insert(error);
                }
            }
            if let Some(error) = first {
                return Err(error);
            }
        }
    }

    fn destroy(&self) {
        self.deferred.resolve();
    }
}

/// The live iteration behind `iterateCallbacks`: each step reads the strategy's plugin list
/// again, and no borrow is held while a callback runs.
pub struct Callbacks<'a> {
    handler: &'a StrategyHandler,
    callback: Callback,
    index: usize,
}

impl<'a> Iterator for Callbacks<'a> {
    type Item = (PluginRef, Option<&'a PluginState>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let plugin = self.handler.strategy.plugins.borrow().get(self.index).cloned()?;
            self.index += 1;
            if plugin.defines(self.callback) {
                let state = self.handler.state_of(&plugin);
                return Some((plugin, state));
            }
        }
    }
}
