//! workbox-routing: `Route` and `Router` (registration per method, the http(s) guard, matching
//! with parameter normalisation, default handlers, route and global catch handlers).

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use mires_js::collections::HashMap;

use crate::host::{self, Request, Url};
use crate::strategies::Strategy;
use crate::workbox_core::{Details, Error, Event, HandlerOptions, Params, Task};

/// A route handler as `normalizeHandler` leaves it: an object with `handle` (a strategy) or a
/// handler callback.
pub enum Handler {
    Strategy(Rc<Strategy>),
    Callback(Box<dyn Fn(HandlerOptions) -> Task<host::Response>>),
}

impl Handler {
    pub fn handle(&self, options: HandlerOptions) -> Task<host::Response> {
        match self {
            Handler::Strategy(strategy) => strategy.handle(options),
            Handler::Callback(callback) => callback(options),
        }
    }
}

/// What a route's match callback sees (`{url, sameOrigin, request}`; no matcher in the subset
/// reads the event Workbox also passes).
pub struct MatchContext<'a> {
    pub url: &'a Url,
    pub same_origin: bool,
    pub request: Request,
}

/// A match callback: `None` for no match, else the parameters (empty for a plain `true`).
pub type Matcher = Box<dyn Fn(&MatchContext<'_>) -> Result<Option<Params>, Error>>;

/// `Route`.
pub struct Route {
    pub handler: Rc<Handler>,
    matcher: Matcher,
    pub method: String,
    catch_handler: RefCell<Option<Rc<Handler>>>,
}

impl Route {
    pub fn new(matcher: Matcher, handler: Rc<Handler>, method: String) -> Self {
        Route { handler, matcher, method, catch_handler: RefCell::new(None) }
    }

    pub fn set_catch_handler(&self, handler: Rc<Handler>) {
        *self.catch_handler.borrow_mut() = Some(handler);
    }
}

/// `Router`.
#[derive(Default)]
pub struct Router {
    routes: RefCell<HashMap<String, Vec<Rc<Route>>>>,
    default_handler_map: RefCell<HashMap<String, Rc<Handler>>>,
    // Shared with pending requests: Workbox reads it when a rejection arrives.
    catch_handler: Rc<RefCell<Option<Rc<Handler>>>>,
}

impl Router {
    /// `handleRequest({request, event})`: `None` when the request is not http(s) or nothing
    /// handles it, else the response task, with the catch handlers attached when there are any.
    pub fn handle_request(&self, request: Request, event: Event) -> Result<Option<Task<host::Response>>, Error> {
        let url = Url::parse(&request.url(), &host::location_href())?;
        if !url.protocol().starts_with("http") {
            return Ok(None);
        }
        let same_origin = url.origin() == host::location_origin();
        let (route, params) = self.find_matching_route(&MatchContext { url: &url, same_origin, request })?.unzip();
        let params = params.flatten();
        let method = request.method();
        let handler = match &route {
            Some(route) => Some(Rc::clone(&route.handler)),
            None => self.default_handler_map.borrow().get(method.as_str()).cloned(),
        };
        let Some(handler) = handler else {
            return Ok(None);
        };
        let response = handler.handle(HandlerOptions { request, event: Rc::clone(&event), params: params.clone() });
        let route_catch = route.as_ref().and_then(|route| route.catch_handler.borrow().clone());
        if route_catch.is_none() && self.catch_handler.borrow().is_none() {
            return Ok(Some(response));
        }
        let global_catch = Rc::clone(&self.catch_handler);
        Ok(Some(Task::spawn(async move {
            let mut error = match response.get().await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            if let Some(route_catch) = route_catch {
                match route_catch.handle(HandlerOptions { request, event: Rc::clone(&event), params }).get().await {
                    Ok(response) => return Ok(response),
                    Err(caught) if caught.is_error() => error = caught,
                    Err(_) => {}
                }
            }
            // The global catch handler is the one set when the rejection arrives; it gets no params.
            let global_catch = global_catch.borrow().clone();
            match global_catch {
                Some(global_catch) => global_catch.handle(HandlerOptions { request, event, params: None }).get().await,
                None => Err(error),
            }
        })))
    }

    /// `findMatchingRoute`: the first route for the request's method that matches, with `{}`
    /// parameters (a plain `true`) normalised to none.
    fn find_matching_route(&self, context: &MatchContext<'_>) -> Result<Option<(Rc<Route>, Option<Params>)>, Error> {
        let routes = self.routes.borrow().get(context.request.method().as_str()).cloned().unwrap_or_default();
        for route in routes {
            if let Some(params) = (route.matcher)(context)? {
                let params = if params.is_empty() { None } else { Some(params) };
                return Ok(Some((route, params)));
            }
        }
        Ok(None)
    }

    pub fn set_default_handler(&self, handler: Rc<Handler>, method: String) {
        self.default_handler_map.borrow_mut().insert(method, handler);
    }

    pub fn set_catch_handler(&self, handler: Rc<Handler>) {
        *self.catch_handler.borrow_mut() = Some(handler);
    }

    /// `registerRoute(route)`: after the routes already registered for its method.
    pub fn register_route(&self, route: Rc<Route>) {
        let mut routes = self.routes.borrow_mut();
        match routes.get_mut(route.method.as_str()) {
            Some(list) => list.push(route),
            None => {
                let method = route.method.clone();
                routes.insert(method, Vec::from([route]));
            }
        }
    }

    pub fn unregister_route(&self, route: &Rc<Route>) -> Result<(), Error> {
        let mut routes = self.routes.borrow_mut();
        let Some(list) = routes.get_mut(route.method.as_str()) else {
            let details = Details::new().text("method", &route.method);
            return Err(Error::workbox("unregister-route-but-not-found-with-method", Some(details)));
        };
        match list.iter().position(|known| Rc::ptr_eq(known, route)) {
            Some(index) => {
                list.remove(index);
                Ok(())
            }
            None => Err(Error::workbox("unregister-route-route-not-registered", None)),
        }
    }
}
