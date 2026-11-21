module Routing exposing (newRoute, setRouteCatchHandler, newRouter, registerRoute, unregisterRoute, setDefaultHandler, setCatchHandler, handleRequest)

{-| workbox-routing: Route and Router (registration per method, the http(s) guard, matching with
parameter normalisation, default handlers, route and global catch handlers).
-}

import Dict
import Host exposing (Request, Response)
import Io exposing (Promise)
import Json.Encode as E
import World exposing (Event, MatchCallbackOptions, Params, Route(..), RouteHandler(..), Router, RouterState, Wb)
import WorkboxCore exposing (workboxError)


{-| `new Route(match, handler, method)`.
-}
newRoute : (MatchCallbackOptions -> Wb (Maybe (Maybe Params))) -> RouteHandler -> String -> Wb Route
newRoute match handler method =
    World.newId |> Io.map (\id -> Route { id = id, match = match, handler = handler, method = method })


setRouteCatchHandler : Route -> RouteHandler -> Wb ()
setRouteCatchHandler (Route route) handler =
    Io.modify (\world -> ( { world | routeCatchHandlers = Dict.insert route.id handler world.routeCatchHandlers }, () ))


newRouter : Wb Router
newRouter =
    World.newId
        |> Io.andThen
            (\id ->
                Io.modify
                    (\world ->
                        ( { world | routers = Dict.insert id { routes = Dict.empty, defaultHandlerMap = Dict.empty, catchHandler = Nothing } world.routers }
                        , { id = id }
                        )
                    )
            )


routerState : Router -> World.World -> RouterState
routerState router world =
    Dict.get router.id world.routers |> Maybe.withDefault { routes = Dict.empty, defaultHandlerMap = Dict.empty, catchHandler = Nothing }


modifyRouter : Router -> (RouterState -> RouterState) -> Wb ()
modifyRouter router change =
    Io.modify (\world -> ( { world | routers = Dict.insert router.id (change (routerState router world)) world.routers }, () ))


{-| `registerRoute(route)`: after the routes already registered for its method.
-}
registerRoute : Router -> Route -> Wb ()
registerRoute router (Route route) =
    modifyRouter router
        (\state -> { state | routes = Dict.insert route.method ((Dict.get route.method state.routes |> Maybe.withDefault []) ++ [ Route route ]) state.routes })


unregisterRoute : Router -> Route -> Wb ()
unregisterRoute router (Route route) =
    Io.get (routerState router >> .routes >> Dict.get route.method)
        |> Io.andThen
            (\found ->
                case found of
                    Nothing ->
                        Io.throw (workboxError "unregister-route-but-not-found-with-method" [ ( "method", E.string route.method ) ])

                    Just routes ->
                        if List.any (\(Route known) -> known.id == route.id) routes then
                            modifyRouter router (\state -> { state | routes = Dict.insert route.method (List.filter (\(Route known) -> known.id /= route.id) routes) state.routes })

                        else
                            Io.throw (Io.WorkboxError "unregister-route-route-not-registered" Nothing)
            )


setDefaultHandler : Router -> RouteHandler -> String -> Wb ()
setDefaultHandler router handler method =
    modifyRouter router (\state -> { state | defaultHandlerMap = Dict.insert method handler state.defaultHandlerMap })


setCatchHandler : Router -> RouteHandler -> Wb ()
setCatchHandler router handler =
    modifyRouter router (\state -> { state | catchHandler = Just handler })


{-| `findMatchingRoute`: the first route for the request's method that matches, with its params.
-}
findMatchingRoute : Router -> MatchCallbackOptions -> Wb (Maybe ( Route, Maybe Params ))
findMatchingRoute router options =
    let
        first routes =
            case routes of
                [] ->
                    Io.succeed Nothing

                (Route route) :: rest ->
                    route.match options
                        |> Io.andThen
                            (\matched ->
                                case matched of
                                    Just params ->
                                        Io.succeed (Just ( Route route, params ))

                                    Nothing ->
                                        first rest
                            )
    in
    Io.get (routerState router >> .routes >> Dict.get options.request.method >> Maybe.withDefault []) |> Io.andThen first


{-| `handleRequest({request, event})`: Nothing when the request is not http(s) or nothing handles
it, else the response promise, with the catch handlers chained when there are any.
-}
handleRequest : Router -> Request -> Event -> Wb (Maybe (Promise Response))
handleRequest router request event =
    Io.get .location
        |> Io.andThen
            (\location ->
                Host.newUrl request.url (Just location.href)
                    |> Io.andThen
                        (\url ->
                            if not (String.startsWith "http" url.protocol) then
                                Io.succeed Nothing

                            else
                                findMatchingRoute router { url = url, sameOrigin = url.origin == location.origin, request = request, event = event }
                                    |> Io.andThen (respond router request event)
                        )
            )


respond : Router -> Request -> Event -> Maybe ( Route, Maybe Params ) -> Wb (Maybe (Promise Response))
respond router request event matched =
    let
        params =
            matched |> Maybe.andThen Tuple.second

        options =
            { request = request, event = event, params = params }
    in
    Io.get
        (\world ->
            ( case matched of
                Just ( Route route, _ ) ->
                    Just route.handler

                Nothing ->
                    Dict.get request.method (routerState router world).defaultHandlerMap
            , matched |> Maybe.andThen (\( Route route, _ ) -> Dict.get route.id world.routeCatchHandlers)
            , (routerState router world).catchHandler
            )
        )
        |> Io.andThen
            (\( handler, routeCatchHandler, catchHandler ) ->
                case handler of
                    Nothing ->
                        Io.succeed Nothing

                    Just routeHandler ->
                        -- A synchronous throw from `handle` becomes a rejected promise.
                        Io.attempt (World.handleWith routeHandler options)
                            |> Io.andThen
                                (\outcome ->
                                    case outcome of
                                        Ok responsePromise ->
                                            Io.succeed responsePromise

                                        Err thrown ->
                                            Io.spawn Host.encodeResponse Host.responseDecoder (Io.throw thrown)
                                )
                            |> Io.andThen
                                (\responsePromise ->
                                    -- Every handler in the subset answers a promise, so only the catch handlers decide.
                                    case ( routeCatchHandler, catchHandler ) of
                                        ( Nothing, Nothing ) ->
                                            Io.succeed (Just responsePromise)

                                        _ ->
                                            Io.spawn Host.encodeResponse Host.responseDecoder (caught router options routeCatchHandler responsePromise)
                                                |> Io.map Just
                                )
            )


{-| The `.catch` Workbox chains: the route's catch handler first (an Error it throws replaces the
original), then the router's, read when the rejection arrives and called without params.
-}
caught : Router -> World.HandlerCallbackOptions -> Maybe RouteHandler -> Promise Response -> Wb Response
caught router options routeCatchHandler responsePromise =
    Io.attempt (Io.await responsePromise)
        |> Io.andThen
            (\outcome ->
                case outcome of
                    Ok response ->
                        Io.succeed response

                    Err original ->
                        (case routeCatchHandler of
                            Just handler ->
                                Io.attempt (World.handleWith handler options |> Io.andThen Io.await)
                                    |> Io.map
                                        (Result.mapError
                                            (\thrown ->
                                                if Io.isError thrown then
                                                    thrown

                                                else
                                                    original
                                            )
                                        )

                            Nothing ->
                                Io.succeed (Err original)
                        )
                            |> Io.andThen
                                (\routed ->
                                    case routed of
                                        Ok response ->
                                            Io.succeed response

                                        Err error ->
                                            Io.get (routerState router >> .catchHandler)
                                                |> Io.andThen
                                                    (\catchHandler ->
                                                        case catchHandler of
                                                            Just handler ->
                                                                World.handleWith handler { options | params = Nothing } |> Io.andThen Io.await

                                                            Nothing ->
                                                                Io.throw error
                                                    )
                                )
            )
