// workbox-routing: Route and Router (registration per method, the http(s) guard, matching with
// parameter normalisation, default handlers, route and global catch handlers).

part of 'workbox.dart';

/// What a route's match callback sees.
class MatchCallbackOptions {
  MatchCallbackOptions(this.url, this.sameOrigin, this.request, this.event);

  final URL url;
  final bool sameOrigin;
  final Request request;
  final ExtendableEvent event;
}

/// `new Route(match, handler, method)`. A match answers a falsy value (null or false) for no
/// match, `true` for a match without parameters, or the [Params] the handler receives.
class Route {
  Route(this.match, this.handler, [this.method = 'GET']);

  final Object? Function(MatchCallbackOptions options) match;
  final RouteHandler handler;
  final String method;
  RouteHandler? _catchHandler;

  RouteHandler? get catchHandler => _catchHandler;

  void setCatchHandler(RouteHandler handler) => _catchHandler = handler;
}

/// `Router`.
class Router {
  final _routes = <String, List<Route>>{};
  final _defaultHandlerMap = <String, RouteHandler>{};
  RouteHandler? _catchHandler;

  /// `handleRequest({request, event})`: null when the request is not http(s) or nothing handles
  /// it, else the response promise, with the catch handlers chained when there are any.
  Future<Response>? handleRequest(Request request, ExtendableEvent event) {
    final url = URL.from(request.url, location.href);
    if (!url.protocol.startsWith('http')) return null;
    final sameOrigin = url.origin == location.origin;
    final (:params, :route) = _findMatchingRoute(MatchCallbackOptions(url, sameOrigin, request, event));
    final handler = route?.handler ?? _defaultHandlerMap[request.method];
    if (handler == null) return null;
    Future<Response> responsePromise;
    try {
      responsePromise = handler.handle(HandlerCallbackOptions(request, event, params));
    } catch (error) {
      responsePromise = Future.error(error);
    }
    // Every handler in the subset answers a Future, so only the catch handlers decide.
    final routeCatchHandler = route?.catchHandler;
    if (_catchHandler == null && routeCatchHandler == null) return responsePromise;
    return responsePromise.catchError((Object caught) async {
      var error = caught;
      if (routeCatchHandler != null) {
        try {
          return await routeCatchHandler.handle(HandlerCallbackOptions(request, event, params));
        } catch (catchErr) {
          if (isError(catchErr)) error = catchErr;
        }
      }
      // The global catch handler is the one set when the rejection arrives; it gets no params.
      final catchHandler = _catchHandler;
      if (catchHandler != null) return catchHandler.handle(HandlerCallbackOptions(request, event));
      throw error;
    });
  }

  /// `findMatchingRoute`: the first route for the request's method that matches. A match of `true`
  /// carries no params.
  ({Params? params, Route? route}) _findMatchingRoute(MatchCallbackOptions options) {
    for (final route in _routes[options.request.method] ?? const <Route>[]) {
      final matchResult = route.match(options);
      if (matchResult != null && matchResult != false) return (params: matchResult is Params ? matchResult : null, route: route);
    }
    return (params: null, route: null);
  }

  void setDefaultHandler(RouteHandler handler, [String method = 'GET']) => _defaultHandlerMap[method] = handler;

  void setCatchHandler(RouteHandler handler) => _catchHandler = handler;

  void registerRoute(Route route) => (_routes[route.method] ??= []).add(route);

  void unregisterRoute(Route route) {
    final routes = _routes[route.method] ??
        (throw WorkboxError('unregister-route-but-not-found-with-method', JSObject()..['method'] = route.method.toJS));
    final routeIndex = routes.indexWhere((known) => identical(known, route));
    if (routeIndex < 0) throw WorkboxError('unregister-route-route-not-registered');
    routes.removeAt(routeIndex);
  }
}
