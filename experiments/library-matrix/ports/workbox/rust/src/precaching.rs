//! workbox-precaching: `PrecacheController`, `PrecacheStrategy`, `PrecacheRoute` with its URL
//! variations, `createCacheKey`, and the precache plugins (cache key, install report, fallback,
//! default cacheability and redirect copy).

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use mires_js::JsValue;
use mires_js::collections::{HashMap, HashSet};

use crate::host::{self, Cache, Request, RequestInit, Response, Url, RegExp};
use crate::routing::{Handler, MatchContext, Route};
use crate::strategies::{plugin, Callback, Cx, Hook, Kind, Mode, Plugin, PluginRef, Strategy, StrategyHandler};
use crate::workbox_core::{copy_response, wait_until, Details, Error, Event, HandlerOptions, Params, Task};

const REVISION_SEARCH_PARAM: &str = "__WB_REVISION__";

/// A precache manifest entry as the caller wrote it (`PrecacheEntry | string`).
pub enum ManifestEntry {
    /// A string: a URL that is its own cache key.
    Url(String),
    /// A `PrecacheEntry` object. `source` is the object itself, which
    /// add-to-cache-list-unexpected-type reports.
    Entry { url: Option<String>, revision: JsValue, integrity: Option<String>, source: JsValue },
    /// `null`, `''` or another falsy value.
    Falsy(JsValue),
}

struct CacheKey {
    cache_key: String,
    url: String,
}

fn unexpected_type(entry: JsValue) -> Error {
    Error::workbox("add-to-cache-list-unexpected-type", Some(Details::new().value("entry", entry)))
}

/// `createCacheKey(entry)`: the URL resolved against the location, and with a truthy revision a
/// cache key that carries it in `__WB_REVISION__` (the host prints a number revision).
fn create_cache_key(entry: &ManifestEntry) -> Result<CacheKey, Error> {
    let base = host::location_href();
    match entry {
        ManifestEntry::Falsy(source) => Err(unexpected_type(*source)),
        ManifestEntry::Url(url) => {
            let href = Url::parse(url, &base)?.href();
            Ok(CacheKey { cache_key: href.clone(), url: href })
        }
        ManifestEntry::Entry { url, revision, source, .. } => {
            let Some(url) = url.as_deref().filter(|url| !url.is_empty()) else {
                return Err(unexpected_type(*source));
            };
            if !revision.is_truthy() {
                let href = Url::parse(url, &base)?.href();
                return Ok(CacheKey { cache_key: href.clone(), url: href });
            }
            let cache_key_url = Url::parse(url, &base)?;
            let original_url = Url::parse(url, &base)?;
            cache_key_url.set_search_param(REVISION_SEARCH_PARAM, *revision);
            Ok(CacheKey { cache_key: cache_key_url.href(), url: original_url.href() })
        }
    }
}

/// `string | Request`, as `matchPrecache` takes it.
pub enum RequestInfo {
    Url(String),
    Request(Request),
}

#[derive(Clone)]
pub struct InstallResult {
    pub updated_urls: Vec<String>,
    pub not_updated_urls: Vec<String>,
}

#[derive(Clone)]
pub struct CleanupResult {
    pub deleted_urls: Vec<String>,
}

/// `PrecacheController`.
pub struct PrecacheController {
    strategy: Rc<Strategy>,
    urls_to_cache_keys: RefCell<HashMap<String, String>>,
    urls_to_cache_modes: RefCell<HashMap<String, &'static str>>,
    cache_keys_to_integrities: RefCell<HashMap<String, String>>,
}

impl PrecacheController {
    /// `new PrecacheController({cacheName, plugins, fallbackToNetwork})`: a PrecacheStrategy
    /// whose plugins end with this controller's cache-key plugin.
    pub fn new(cache_name: String, plugins: Vec<PluginRef>, fallback_to_network: bool) -> Rc<Self> {
        Rc::new_cyclic(|controller| {
            let mut plugins = plugins;
            plugins.push(plugin(PrecacheCacheKeyPlugin { controller: Weak::clone(controller) }));
            PrecacheController {
                strategy: PrecacheStrategy::create(cache_name, plugins, fallback_to_network),
                urls_to_cache_keys: RefCell::new(HashMap::new()),
                urls_to_cache_modes: RefCell::new(HashMap::new()),
                cache_keys_to_integrities: RefCell::new(HashMap::new()),
            }
        })
    }

    pub fn strategy(&self) -> &Rc<Strategy> {
        &self.strategy
    }

    /// `addToCacheList(entries)`: entry by entry, so the entries before a failing one stay added.
    /// Like the production build it warns on the console about revisionless entries.
    pub fn add_to_cache_list(&self, entries: &[ManifestEntry]) -> Result<(), Error> {
        let mut urls_to_warn_about: Vec<String> = Vec::new();
        for entry in entries {
            match entry {
                ManifestEntry::Url(url) => urls_to_warn_about.push(url.clone()),
                ManifestEntry::Entry { url, revision, .. } if revision.is_undefined() => urls_to_warn_about.push(url.clone().unwrap_or_default()),
                _ => {}
            }
            let CacheKey { cache_key, url } = create_cache_key(entry)?;
            let cache_mode = match entry {
                ManifestEntry::Entry { revision, .. } if revision.is_truthy() => "reload",
                _ => "default",
            };
            let first_entry = self.urls_to_cache_keys.borrow().get(url.as_str()).cloned();
            if let Some(first_entry) = first_entry.filter(|first| *first != cache_key) {
                let details = Details::new().text("firstEntry", &first_entry).text("secondEntry", &cache_key);
                return Err(Error::workbox("add-to-cache-list-conflicting-entries", Some(details)));
            }
            if let ManifestEntry::Entry { integrity: Some(integrity), .. } = entry {
                if !integrity.is_empty() {
                    let known = self.cache_keys_to_integrities.borrow().get(cache_key.as_str()).cloned();
                    if known.is_some_and(|known| known != *integrity) {
                        return Err(Error::workbox("add-to-cache-list-conflicting-integrities", Some(Details::new().text("url", &url))));
                    }
                    self.cache_keys_to_integrities.borrow_mut().insert(cache_key.clone(), integrity.clone());
                }
            }
            self.urls_to_cache_keys.borrow_mut().insert(url.clone(), cache_key);
            self.urls_to_cache_modes.borrow_mut().insert(url, cache_mode);
            if !urls_to_warn_about.is_empty() {
                let mut message = String::from("Workbox is precaching URLs without revision info: ");
                for (index, url) in urls_to_warn_about.iter().enumerate() {
                    if index > 0 {
                        message.push_str(", ");
                    }
                    message.push_str(url);
                }
                message.push_str("\nThis is generally NOT safe. Learn more at https://bit.ly/wb-precache");
                host::warn(&message);
            }
        }
        Ok(())
    }

    /// `install(event)`: precaches each URL in turn through the strategy, under an install
    /// report plugin, and extends the event's lifetime until it is done.
    pub fn install(self: &Rc<Self>, event: Event) -> Task<InstallResult> {
        let controller = Rc::clone(self);
        let lifetime = Rc::clone(&event);
        let task = Task::spawn(async move {
            let report = Rc::new(InstallReport::default());
            controller.strategy.plugins.borrow_mut().push(plugin(PrecacheInstallReportPlugin { report: Rc::clone(&report) }));
            let entries: Vec<(String, String)> = controller.urls_to_cache_keys.borrow().iter().map(|(url, key)| (String::from(url), key.clone())).collect();
            for (url, cache_key) in entries {
                let integrity = controller.cache_keys_to_integrities.borrow().get(cache_key.as_str()).cloned();
                let cache_mode = controller.urls_to_cache_modes.borrow().get(url.as_str()).copied();
                let init = RequestInit { integrity: integrity.as_deref(), cache: cache_mode, credentials: Some("same-origin"), ..RequestInit::default() };
                let request = Request::new(&url, &init)?;
                let params = Params { cache_key: Some(cache_key), integrity: None };
                let (response_done, handler_done) = controller.strategy.handle_all(HandlerOptions { request, event: Rc::clone(&event), params: Some(params) });
                // Promise.all: the response's failure, else the completion's.
                response_done.get().await?;
                handler_done.get().await?;
            }
            let updated_urls = report.updated_urls.borrow().clone();
            let not_updated_urls = report.not_updated_urls.borrow().clone();
            Ok(InstallResult { updated_urls, not_updated_urls })
        });
        wait_until(&lifetime, task)
    }

    /// `activate(event)`: deletes every request in the precache whose URL is no current cache key.
    pub fn activate(self: &Rc<Self>, event: Event) -> Task<CleanupResult> {
        let controller = Rc::clone(self);
        let task = Task::spawn(async move {
            let cache = Cache::open(&controller.strategy.cache_name).await?;
            let currently_cached = cache.keys().await?;
            let mut expected: HashSet<String> = HashSet::new();
            for key in controller.urls_to_cache_keys.borrow().iter().map(|(_, key)| key.clone()) {
                expected.insert(key);
            }
            let mut deleted_urls = Vec::new();
            for request in currently_cached {
                let url = request.url();
                if !expected.contains(url.as_str()) {
                    cache.delete(request).await?;
                    deleted_urls.push(url);
                }
            }
            Ok(CleanupResult { deleted_urls })
        });
        wait_until(&event, task)
    }

    /// `getURLsToCacheKeys()`, in insertion order.
    pub fn urls_to_cache_keys(&self) -> Vec<(String, String)> {
        self.urls_to_cache_keys.borrow().iter().map(|(url, key)| (String::from(url), key.clone())).collect()
    }

    /// `getCacheKeyForURL(url)`: the exact URL resolved against the location, without variations.
    pub fn cache_key_for_url(&self, url: &str) -> Result<Option<String>, Error> {
        let url = Url::parse(url, &host::location_href())?;
        Ok(self.urls_to_cache_keys.borrow().get(url.href().as_str()).cloned())
    }

    pub fn integrity_for_cache_key(&self, cache_key: &str) -> Option<String> {
        self.cache_keys_to_integrities.borrow().get(cache_key).cloned()
    }

    /// `matchPrecache(request)`: opens the precache only for a URL in the manifest.
    pub async fn match_precache(&self, request: RequestInfo) -> Result<Option<Response>, Error> {
        let url = match request {
            RequestInfo::Request(request) => request.url(),
            RequestInfo::Url(url) => url,
        };
        match self.cache_key_for_url(&url)? {
            Some(cache_key) => Cache::open(&self.strategy.cache_name).await?.match_url(&cache_key).await,
            None => Ok(None),
        }
    }

    /// `createHandlerBoundToURL(url)`: a handler callback that answers any request with this
    /// precached URL's entry.
    pub fn create_handler_bound_to_url(self: &Rc<Self>, url: &str) -> Result<Handler, Error> {
        let Some(cache_key) = self.cache_key_for_url(url)? else {
            return Err(Error::workbox("non-precached-url", Some(Details::new().text("url", url))));
        };
        let (url, strategy) = (String::from(url), Rc::clone(&self.strategy));
        Ok(Handler::Callback(Box::new(move |options: HandlerOptions| {
            let request = match Request::new(&url, &RequestInit::default()) {
                Ok(request) => request,
                Err(error) => return Task::spawn(async move { Err(error) }),
            };
            // `{cacheKey, ...options.params}`.
            let params = match options.params {
                Some(params) => Params { cache_key: params.cache_key.or_else(|| Some(cache_key.clone())), integrity: params.integrity },
                None => Params { cache_key: Some(cache_key.clone()), integrity: None },
            };
            strategy.handle(HandlerOptions { request, event: options.event, params: Some(params) })
        })))
    }
}

/// The PrecacheStrategy part of a Strategy: `fallbackToNetwork` and the class's two static
/// plugins (one instance each per strategy; each list holds only its own).
pub struct PrecacheStrategy {
    fallback_to_network: bool,
    default_precache_cacheability: PluginRef,
    copy_redirected_cacheable_responses: PluginRef,
}

impl PrecacheStrategy {
    /// `new PrecacheStrategy({cacheName, plugins, fallbackToNetwork})`, which appends the
    /// redirect-copy plugin.
    pub fn create(cache_name: String, plugins: Vec<PluginRef>, fallback_to_network: bool) -> Rc<Strategy> {
        let copy = plugin(CopyRedirectedCacheableResponses);
        let mut plugins = plugins;
        plugins.push(Rc::clone(&copy));
        let kind = Kind::Precache(PrecacheStrategy {
            fallback_to_network,
            default_precache_cacheability: plugin(DefaultPrecacheCacheability),
            copy_redirected_cacheable_responses: copy,
        });
        Strategy::new(kind, cache_name, plugins)
    }

    /// `_handle`: the cache, then install-time population, then the network fallback.
    pub async fn handle(&self, strategy: &Strategy, request: Request, handler: &Rc<StrategyHandler>) -> Result<Response, Error> {
        if let Some(response) = handler.cache_match(request).await? {
            return Ok(response);
        }
        if handler.event.event_type() == "install" {
            return self.handle_install(strategy, request, handler).await;
        }
        self.handle_fetch(strategy, request, handler).await
    }

    /// `_handleFetch`: the network (with the manifest's integrity unless the request is
    /// no-cors), and a cache repair only when integrity vouches for the response.
    async fn handle_fetch(&self, strategy: &Strategy, request: Request, handler: &Rc<StrategyHandler>) -> Result<Response, Error> {
        if !self.fallback_to_network {
            let details = Details::new().text("cacheName", &strategy.cache_name).text("url", &request.url());
            return Err(Error::workbox("missing-precache-entry", Some(details)));
        }
        let params = handler.params.clone().unwrap_or_default();
        let in_manifest = params.integrity.filter(|integrity| !integrity.is_empty());
        let in_request = request.integrity();
        let no_integrity_conflict = in_request.is_empty() || Some(&in_request) == in_manifest.as_ref();
        let cors = request.mode() != "no-cors";
        let integrity = if !cors { None } else if !in_request.is_empty() { Some(in_request.as_str()) } else { in_manifest.as_deref() };
        let response = handler.fetch(request.with(&RequestInit { integrity, ..RequestInit::default() })?).await?;
        if in_manifest.is_some() && no_integrity_conflict && cors {
            self.use_default_cacheability_plugin_if_needed(strategy);
            handler.cache_put(request, response.duplicate()).await?;
        }
        Ok(response)
    }

    /// `_handleInstall`: fetch, then cache, and fail the install when the response may not be
    /// cached.
    async fn handle_install(&self, strategy: &Strategy, request: Request, handler: &Rc<StrategyHandler>) -> Result<Response, Error> {
        self.use_default_cacheability_plugin_if_needed(strategy);
        let response = handler.fetch(request).await?;
        if !handler.cache_put(request, response.duplicate()).await? {
            let details = Details::new().text("url", &request.url()).number("status", response.status());
            return Err(Error::workbox("bad-precaching-response", Some(details)));
        }
        Ok(response)
    }

    /// `_useDefaultCacheabilityPluginIfNeeded`: exactly one cacheWillUpdate besides the redirect
    /// copy, adding the default one when there is none and dropping it when there are more.
    fn use_default_cacheability_plugin_if_needed(&self, strategy: &Strategy) {
        let mut plugins = strategy.plugins.borrow_mut();
        let mut default_plugin_index = None;
        let mut cache_will_update_plugin_count = 0;
        for (index, plugin) in plugins.iter().enumerate() {
            if Rc::ptr_eq(plugin, &self.copy_redirected_cacheable_responses) {
                continue;
            }
            if Rc::ptr_eq(plugin, &self.default_precache_cacheability) {
                default_plugin_index = Some(index);
            }
            if plugin.defines(Callback::CacheWillUpdate) {
                cache_will_update_plugin_count += 1;
            }
        }
        if cache_will_update_plugin_count == 0 {
            plugins.push(Rc::clone(&self.default_precache_cacheability));
        } else if cache_will_update_plugin_count > 1 {
            if let Some(index) = default_plugin_index {
                plugins.remove(index);
            }
        }
    }
}

/// `PrecacheStrategy.defaultPrecacheCacheabilityPlugin`: statuses below 400.
struct DefaultPrecacheCacheability;

impl Plugin for DefaultPrecacheCacheability {
    fn defines(&self, callback: Callback) -> bool {
        callback == Callback::CacheWillUpdate
    }
    fn cache_will_update<'a>(&'a self, _: Cx<'a>, _: Request, response: Response) -> Hook<'a, Option<Response>> {
        Box::pin(async move { Ok(if response.status() >= 400 { None } else { Some(response) }) })
    }
}

/// `PrecacheStrategy.copyRedirectedCacheableResponsesPlugin`.
struct CopyRedirectedCacheableResponses;

impl Plugin for CopyRedirectedCacheableResponses {
    fn defines(&self, callback: Callback) -> bool {
        callback == Callback::CacheWillUpdate
    }
    fn cache_will_update<'a>(&'a self, _: Cx<'a>, _: Request, response: Response) -> Hook<'a, Option<Response>> {
        Box::pin(async move { Ok(Some(if response.redirected() { copy_response(response).await? } else { response })) })
    }
}

/// `PrecacheCacheKeyPlugin`: the route's cache key, else the manifest's key for the exact URL.
struct PrecacheCacheKeyPlugin {
    controller: Weak<PrecacheController>,
}

impl Plugin for PrecacheCacheKeyPlugin {
    fn defines(&self, callback: Callback) -> bool {
        callback == Callback::CacheKeyWillBeUsed
    }
    fn cache_key_will_be_used<'a>(&'a self, _: Cx<'a>, _: Mode, request: Request, params: Option<&'a Params>) -> Hook<'a, Request> {
        Box::pin(async move {
            let cache_key = match params.and_then(|params| params.cache_key.clone()).filter(|key| !key.is_empty()) {
                Some(cache_key) => Some(cache_key),
                None => self.controller.upgrade().expect("the controller owns its strategy").cache_key_for_url(&request.url())?,
            };
            match cache_key {
                Some(cache_key) => Request::new(&cache_key, &RequestInit { headers: Some(request.headers()), ..RequestInit::default() }),
                None => Ok(request),
            }
        })
    }
}

/// What one install reports.
#[derive(Default)]
struct InstallReport {
    updated_urls: RefCell<Vec<String>>,
    not_updated_urls: RefCell<Vec<String>>,
}

/// `PrecacheInstallReportPlugin`: an install-time cache hit is not updated, a miss is.
struct PrecacheInstallReportPlugin {
    report: Rc<InstallReport>,
}

impl Plugin for PrecacheInstallReportPlugin {
    fn defines(&self, callback: Callback) -> bool {
        matches!(callback, Callback::HandlerWillStart | Callback::CachedResponseWillBeUsed)
    }
    fn handler_will_start<'a>(&'a self, cx: Cx<'a>, request: Request) -> Hook<'a, ()> {
        Box::pin(async move {
            if let Some(state) = cx.state {
                state.set("originalRequest", request.value());
            }
            Ok(())
        })
    }
    fn cached_response_will_be_used<'a>(&'a self, cx: Cx<'a>, _: &'a str, _: Request, cached: Option<Response>) -> Hook<'a, Option<Response>> {
        Box::pin(async move {
            if cx.event.event_type() == "install" {
                let original = cx.state.map(|state| state.get("originalRequest")).filter(|original| !original.is_undefined());
                if let Some(original) = original.map(Request::from_value) {
                    let urls = if cached.is_some() { &self.report.not_updated_urls } else { &self.report.updated_urls };
                    urls.borrow_mut().push(original.url());
                }
            }
            Ok(cached)
        })
    }
}

/// `PrecacheFallbackPlugin`: a failed strategy answers with a precached URL, when it is there.
pub struct PrecacheFallbackPlugin {
    fallback_url: String,
    precache_controller: Rc<PrecacheController>,
}

impl PrecacheFallbackPlugin {
    pub fn new(fallback_url: String, precache_controller: Rc<PrecacheController>) -> Self {
        PrecacheFallbackPlugin { fallback_url, precache_controller }
    }
}

impl Plugin for PrecacheFallbackPlugin {
    fn defines(&self, callback: Callback) -> bool {
        callback == Callback::HandlerDidError
    }
    fn handler_did_error<'a>(&'a self, _: Cx<'a>, _: Request, _: Error) -> Hook<'a, Option<Response>> {
        Box::pin(self.precache_controller.match_precache(RequestInfo::Url(self.fallback_url.clone())))
    }
}

/// A `urlManipulation` callback: more URLs to try, given the URL without ignored parameters.
pub type UrlManipulation = Box<dyn Fn(&Url) -> Result<Vec<Url>, Error>>;

/// `PrecacheRouteOptions`. `directory_index` is `None` when the option is falsy.
pub struct PrecacheRouteOptions {
    pub directory_index: Option<String>,
    pub ignore_url_parameters_matching: Vec<RegExp>,
    pub clean_urls: bool,
    pub url_manipulation: Option<UrlManipulation>,
}

impl PrecacheRouteOptions {
    /// The defaults: `index.html`, `[/^utm_/, /^fbclid$/]`, clean URLs, no manipulation.
    pub fn defaults() -> Result<Self, Error> {
        Ok(PrecacheRouteOptions {
            directory_index: Some(String::from("index.html")),
            ignore_url_parameters_matching: Vec::from([RegExp::new("^utm_")?, RegExp::new("^fbclid$")?]),
            clean_urls: true,
            url_manipulation: None,
        })
    }
}

/// `removeIgnoredSearchParams(url, patterns)`, on the URL itself.
fn remove_ignored_search_params(url: &Url, patterns: &[RegExp]) {
    for name in url.search_param_names() {
        if patterns.iter().any(|pattern| pattern.test(&name)) {
            url.delete_search_param(&name);
        }
    }
}

/// `generateURLVariations(url, options)`, consumed as the route does: each variation in order
/// until `found` accepts one. Later variations (and `urlManipulation`) are not computed.
fn find_variation<T>(url: &str, options: &PrecacheRouteOptions, mut found: impl FnMut(&str) -> Option<T>) -> Result<Option<T>, Error> {
    let url_object = Url::parse(url, &host::location_href())?;
    url_object.set_hash("");
    if let Some(hit) = found(&url_object.href()) {
        return Ok(Some(hit));
    }
    // The URL object itself loses its ignored parameters; urlManipulation sees it that way.
    remove_ignored_search_params(&url_object, &options.ignore_url_parameters_matching);
    let without_ignored = url_object.href();
    if let Some(hit) = found(&without_ignored) {
        return Ok(Some(hit));
    }
    if let Some(directory_index) = &options.directory_index {
        if url_object.pathname().ends_with('/') {
            let directory_url = Url::absolute(&without_ignored)?;
            directory_url.set_pathname(&(directory_url.pathname() + directory_index));
            if let Some(hit) = found(&directory_url.href()) {
                return Ok(Some(hit));
            }
        }
    }
    if options.clean_urls {
        let clean_url = Url::absolute(&without_ignored)?;
        clean_url.set_pathname(&(clean_url.pathname() + ".html"));
        if let Some(hit) = found(&clean_url.href()) {
            return Ok(Some(hit));
        }
    }
    if let Some(url_manipulation) = &options.url_manipulation {
        for url_to_attempt in url_manipulation(&url_object)? {
            if let Some(hit) = found(&url_to_attempt.href()) {
                return Ok(Some(hit));
            }
        }
    }
    Ok(None)
}

/// `new PrecacheRoute(controller, options)`: a GET route to the controller's strategy that
/// matches any variation of the request URL in the manifest, with `{cacheKey, integrity}`.
pub fn precache_route(controller: Rc<PrecacheController>, options: PrecacheRouteOptions) -> Route {
    let handler = Rc::new(Handler::Strategy(Rc::clone(controller.strategy())));
    let matcher = move |context: &MatchContext<'_>| {
        let urls_to_cache_keys = controller.urls_to_cache_keys.borrow();
        find_variation(&context.request.url(), &options, |possible_url| {
            urls_to_cache_keys.get(possible_url).filter(|key| !key.is_empty()).map(|cache_key| Params {
                cache_key: Some(cache_key.clone()),
                integrity: controller.integrity_for_cache_key(cache_key),
            })
        })
    };
    Route::new(Box::new(matcher), handler, String::from("GET"))
}
