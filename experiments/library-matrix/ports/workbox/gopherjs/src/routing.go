// workbox-routing 7.4.1: Route and Router (routes per method, the http(s) guard, matching with
// parameter normalisation, default handlers, route and global catch handlers).

package main

import "strings"

// Handler is a route handler as normalizeHandler leaves it: an object with handle (a strategy), or
// a handler callback.
type Handler interface {
	Handle(options HandlerOptions) *Task[Response]
}

// HandlerFunc is a handler callback.
type HandlerFunc func(options HandlerOptions) *Task[Response]

func (f HandlerFunc) Handle(options HandlerOptions) *Task[Response] { return f(options) }

// MatchOptions is what a match callback receives. No matcher in the subset reads the event Workbox
// also passes.
type MatchOptions struct {
	URL        URL
	SameOrigin bool
	Request    Request
}

// MatchCallback answers whether the route matches and, if so, its parameters: nil for a match that
// is a plain true. It can throw, as a JavaScript match callback can.
type MatchCallback func(options MatchOptions) (matched bool, params *Params, err error)

// Route is new Route(match, handler, method).
type Route struct {
	handler      Handler
	match        MatchCallback
	method       string
	catchHandler Handler
}

// newRoute: an absent method (nil) is GET.
func newRoute(match MatchCallback, handler Handler, method *string) *Route {
	return &Route{handler: handler, match: match, method: orGET(method)}
}

func orGET(method *string) string {
	if method == nil {
		return "GET"
	}
	return *method
}

func (r *Route) SetCatchHandler(handler Handler) { r.catchHandler = handler }

// Router is new Router().
type Router struct {
	routes            map[string][]*Route
	defaultHandlerMap map[string]Handler
	catchHandler      Handler
}

func newRouter() *Router {
	return &Router{routes: map[string][]*Route{}, defaultHandlerMap: map[string]Handler{}}
}

// findMatchingRoute is the first route for the request's method that matches, with its
// parameters.
func (r *Router) findMatchingRoute(options MatchOptions) (*Route, *Params, error) {
	for _, route := range r.routes[options.Request.Method()] {
		matched, params, err := route.match(options)
		if err != nil {
			return nil, nil, err
		}
		if matched {
			return route, params, nil
		}
	}
	return nil, nil, nil
}

// invokeHandler runs handler.Handle and turns a synchronous JS throw into a failed Task.
func invokeHandler(handler Handler, options HandlerOptions) (task *Task[Response]) {
	if err := catch(func() { task = handler.Handle(options) }); err != nil {
		return failed[Response](err)
	}
	return task
}

// HandleRequest is handleRequest({request, event}): nil when the request is not http(s) or nothing
// handles it, else the response, behind the catch handlers when there are any. A synchronous throw
// (a match callback's) is the error.
func (r *Router) HandleRequest(request Request, event ExtendableEvent) (*Task[Response], error) {
	url, err := parseURL(request.URL(), locationHref())
	if err != nil {
		return nil, err
	}
	if !strings.HasPrefix(url.Protocol(), "http") {
		return nil, nil
	}
	sameOrigin := url.Origin() == locationOrigin()
	route, params, err := r.findMatchingRoute(MatchOptions{URL: url, SameOrigin: sameOrigin, Request: request})
	if err != nil {
		return nil, err
	}
	var handler Handler
	if route != nil {
		handler = route.handler
	} else if defaultHandler, ok := r.defaultHandlerMap[request.Method()]; ok {
		handler = defaultHandler
	}
	if handler == nil {
		return nil, nil
	}
	response := invokeHandler(handler, HandlerOptions{Request: request, Event: event, Params: params})
	var catchHandler Handler
	if route != nil {
		catchHandler = route.catchHandler
	}
	if r.catchHandler == nil && catchHandler == nil {
		return response, nil
	}
	// The global catch handler is the one set when the rejection arrives; it gets no params.
	fallBack := func(err error, settle func(Response, error)) {
		if r.catchHandler == nil {
			settle(Response{}, err)
			return
		}
		invokeHandler(r.catchHandler, HandlerOptions{Request: request, Event: event}).then(settle)
	}
	return start(func(settle func(Response, error)) {
		response.then(func(value Response, err error) {
			switch {
			case err == nil:
				settle(value, nil)
			case catchHandler == nil:
				fallBack(err, settle)
			default:
				invokeHandler(catchHandler, HandlerOptions{Request: request, Event: event, Params: params}).then(func(value Response, catchErr error) {
					switch {
					case catchErr == nil:
						settle(value, nil)
					case isError(catchErr):
						// An Error from the route's catch handler replaces the original.
						fallBack(catchErr, settle)
					default:
						fallBack(err, settle)
					}
				})
			}
		})
	}), nil
}

// SetDefaultHandler is setDefaultHandler(handler, method); an absent method (nil) is GET.
func (r *Router) SetDefaultHandler(handler Handler, method *string) {
	r.defaultHandlerMap[orGET(method)] = handler
}

func (r *Router) SetCatchHandler(handler Handler) { r.catchHandler = handler }

// RegisterRoute adds the route after the routes already registered for its method.
func (r *Router) RegisterRoute(route *Route) {
	r.routes[route.method] = append(r.routes[route.method], route)
}

func (r *Router) UnregisterRoute(route *Route) error {
	routes, ok := r.routes[route.method]
	if !ok {
		return newWorkboxError("unregister-route-but-not-found-with-method", newObject("method", route.method))
	}
	for index, known := range routes {
		if known == route {
			r.routes[route.method] = append(routes[:index:index], routes[index+1:]...)
			return nil
		}
	}
	return newWorkboxError("unregister-route-route-not-registered", nil)
}
