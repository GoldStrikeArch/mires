module Scenario exposing (Start, startDecoder, run)

{-| The adapter: what `libraries/workbox/reference.mjs` does, on the port's Workbox. One call runs
one script of Workbox API calls (CONTRACT.md "Steps") with the fixture plugins, builds the
canonical output, and deletes the script's caches on every path.

Each step is read from the input in place with elm/json. The fixture plugins are Elm values like
Workbox's own plugins.

-}

import Dict exposing (Dict)
import Host exposing (Request, Response)
import Io exposing (Io, Thrown(..))
import Json.Decode as D
import Json.Encode as E
import Precaching exposing (PrecacheEntry(..))
import Routing
import Strategies
import World exposing (Event(..), Plugin(..), PrecacheController, Route, RouteHandler(..), Router, StateRef, Wb)


fixturePath : String
fixturePath =
    "experiments/library-matrix/libraries/workbox/"


{-| Well-formed SRI metadata that matches no fixture, so the fetch fails integrity.
-}
wrongIntegrity : String
wrongIntegrity =
    "sha256-" ++ String.repeat 43 "A" ++ "="


{-| What the host starts a call with: the steps, the cache prefix of service-worker mode, whether
a native event is given, and the native request.
-}
type alias Start =
    { steps : List D.Value
    , cachePrefix : Maybe String
    , native : Bool
    , request : Maybe Request
    }


startDecoder : D.Decoder Start
startDecoder =
    D.map4 Start
        (D.at [ "input", "steps" ] (D.list D.value))
        (D.maybe (D.at [ "input", "cachePrefix" ] D.string))
        (D.field "native" D.bool)
        (D.field "request" (D.nullable Host.requestDecoder))


{-| The ids a script names.
-}
type alias Scenario =
    { start : Start
    , controllers : Dict String PrecacheController
    , routers : Dict String Router
    , routes : Dict String Route
    }



-- Reading a step


field : String -> D.Value -> Maybe D.Value
field key value =
    D.decodeValue (D.field key D.value) value |> Result.toMaybe


text : String -> D.Value -> Maybe String
text key value =
    D.decodeValue (D.field key D.string) value |> Result.toMaybe


list : String -> D.Value -> List D.Value
list key value =
    D.decodeValue (D.field key (D.list D.value)) value |> Result.withDefault []


required : String -> D.Value -> String
required key value =
    text key value |> Maybe.withDefault ""


lookup : String -> Dict String a -> Wb a
lookup id known =
    case Dict.get id known of
        Just found ->
            Io.succeed found

        Nothing ->
            Io.exit (Err ("the script names an unknown id " ++ id))



-- Output


relative : String -> Wb String
relative url =
    Io.get (\world -> stripBase world.fixture.base url)


stripBase : String -> String -> String
stripBase base url =
    if String.startsWith base url then
        String.dropLeft (String.length base) url

    else
        url


cacheLabel : String -> String -> String
cacheLabel prefix name =
    if String.startsWith (prefix ++ "-") name then
        String.dropLeft (String.length prefix + 1) name

    else
        name


status : Maybe Response -> E.Value
status =
    Maybe.map (.status >> E.int) >> Maybe.withDefault E.null


{-| `{status, url, body}` of a response, reading its body.
-}
record : Response -> Wb E.Value
record response =
    relative response.url
        |> Io.andThen
            (\url ->
                Host.responseText response
                    |> Io.map (\body -> E.object [ ( "status", E.int response.status ), ( "url", E.string url ), ( "body", E.string body ) ])
            )


{-| `{error, message}` for a WorkboxError, `{error}` for another Error, `{thrown}` for a thrown
non-Error.
-}
failure : World.Fixture -> Thrown -> E.Value
failure fixture thrown =
    case thrown of
        WorkboxError name details ->
            E.object
                [ ( "error", E.string name )
                , ( "message", E.string (Io.workboxMessage name details |> String.replace fixture.base "" |> String.replace (fixture.prefix ++ "-") "") )
                ]

        HostError error ->
            if error.isError then
                E.object [ ( "error", E.string error.name ) ]

            else
                E.object [ ( "thrown", E.string error.text ) ]


{-| Every cache of the scenario, in CacheStorage order, with each entry's response record.
-}
dump : Wb E.Value
dump =
    Io.get (.fixture >> .prefix)
        |> Io.andThen
            (\prefix ->
                Host.cachesKeys
                    |> Io.andThen
                        (\names ->
                            Io.sequence
                                (names
                                    |> List.filter (String.startsWith (prefix ++ "-"))
                                    |> List.map
                                        (\name ->
                                            Host.cachesOpen name
                                                |> Io.andThen
                                                    (\cache ->
                                                        Host.cacheKeys cache
                                                            |> Io.andThen (Io.sequence << List.map (entry cache))
                                                            |> Io.map (\entries -> E.list identity [ E.string (cacheLabel prefix name), E.list identity entries ])
                                                    )
                                        )
                                )
                        )
                    |> Io.map (E.list identity)
            )


entry : Host.Cache -> Request -> Wb E.Value
entry cache request =
    Host.cacheMatch cache request
        |> Io.andThen
            (\found ->
                case found of
                    Just response ->
                        relative request.url |> Io.andThen (\url -> record response |> Io.map (\responseRecord -> E.list identity [ E.string url, responseRecord ]))

                    Nothing ->
                        Io.throw (HostError { name = "TypeError", message = "an entry without a response", isError = True, text = "TypeError: an entry without a response" })
            )



-- Running a script


{-| Runs every step, reports, and deletes the scenario's caches whatever happened.
-}
run : Start -> Wb E.Value
run start =
    Host.locationHref
        |> Io.andThen
            (\href ->
                Host.locationOrigin
                    |> Io.andThen
                        (\origin ->
                            Host.newUrl "./" (Just href)
                                |> Io.andThen
                                    (\baseUrl ->
                                        (case start.cachePrefix of
                                            Just prefix ->
                                                Io.succeed prefix

                                            Nothing ->
                                                Host.randomUUID |> Io.map ((++) "mires-workbox-")
                                        )
                                            |> Io.andThen
                                                (\prefix ->
                                                    Io.modify
                                                        (\world ->
                                                            ( { world
                                                                | location = { href = href, origin = origin }
                                                                , fixture = { base = baseUrl.href, prefix = prefix, source = "alpha", traces = [], logs = Dict.empty, events = [] }
                                                              }
                                                            , ()
                                                            )
                                                        )
                                                )
                                    )
                        )
            )
        |> Io.andThen (\() -> report { start = start, controllers = Dict.empty, routers = Dict.empty, routes = Dict.empty } |> Io.finally (cleanup start))


cleanup : Start -> Wb ()
cleanup start =
    case start.cachePrefix of
        Just _ ->
            Io.succeed ()

        Nothing ->
            Io.get (.fixture >> .prefix)
                |> Io.andThen
                    (\prefix ->
                        Host.cachesKeys
                            |> Io.andThen (Io.forEach (\name -> if String.startsWith (prefix ++ "-") name then Host.cachesDelete name |> Io.map (\_ -> ()) else Io.succeed ()))
                    )


report : Scenario -> Wb E.Value
report scenario =
    Io.foldl
        (\stepValue ( current, results ) ->
            Io.attempt (step current stepValue)
                |> Io.andThen
                    (\outcome ->
                        case outcome of
                            Ok ( next, result ) ->
                                Io.succeed ( next, result :: results )

                            Err thrown ->
                                Io.get (\world -> ( current, failure world.fixture thrown :: results ))
                    )
        )
        ( scenario, [] )
        scenario.start.steps
        |> Io.andThen
            (\( _, results ) ->
                dump
                    |> Io.andThen
                        (\caches ->
                            Io.get
                                (\world ->
                                    E.object
                                        [ ( "results", E.list identity (List.reverse results) )
                                        , ( "events", E.list identity (List.reverse world.fixture.events) )
                                        , ( "handlers", E.list (traceRecord world.fixture) (List.reverse world.fixture.traces) )
                                        , ( "caches", caches )
                                        ]
                                )
                        )
            )


traceRecord : World.Fixture -> World.Trace -> E.Value
traceRecord fixture trace =
    E.list identity [ E.string trace.label, E.string trace.url, E.list identity (Dict.get trace.log fixture.logs |> Maybe.withDefault [] |> List.reverse) ]


{-| One step: the scenario after it, and its result.
-}
step : Scenario -> D.Value -> Wb ( Scenario, E.Value )
step scenario value =
    let
        unchanged =
            Io.map (\result -> ( scenario, result ))

        done =
            Io.map (\() -> ( scenario, E.null ))

        controller key =
            lookup (required key value) scenario.controllers

        router =
            lookup (required "router" value) scenario.routers

        method =
            text "method" value |> Maybe.withDefault "GET"
    in
    case required "op" value of
        "network" ->
            Io.modify (\world -> ( { world | fixture = (\fixture -> { fixture | source = required "source" value }) world.fixture }, () )) |> done

        "controller" ->
            plugins scenario (list "plugins" value)
                |> Io.andThen
                    (\specs ->
                        cacheName (required "cacheName" value)
                            |> Io.andThen (\name -> Precaching.newPrecacheController name specs (D.decodeValue (D.field "fallbackToNetwork" D.bool) value /= Ok False))
                    )
                |> Io.map (\created -> ( { scenario | controllers = Dict.insert (required "id" value) created scenario.controllers }, E.null ))

        "addToCacheList" ->
            controller "controller" |> Io.andThen (\found -> Precaching.addToCacheList found (List.map precacheEntry (list "entries" value))) |> done

        "addPlugins" ->
            controller "controller"
                |> Io.andThen (\found -> Io.sequence (List.map (plugin scenario) (list "plugins" value)) |> Io.andThen (\added -> Strategies.modifyPlugins found.strategy (\current -> current ++ added)))
                |> done

        "install" ->
            controller "controller"
                |> Io.andThen (\found -> lifetime scenario.start "install" (\event -> Precaching.install found event |> Io.andThen Io.await))
                |> Io.andThen (\result -> urls result.updatedURLs |> Io.andThen (\updated -> urls result.notUpdatedURLs |> Io.map (\notUpdated -> E.object [ ( "updatedURLs", updated ), ( "notUpdatedURLs", notUpdated ) ])))
                |> unchanged

        "activate" ->
            controller "controller"
                |> Io.andThen (\found -> lifetime scenario.start "activate" (\event -> Precaching.activate found event |> Io.andThen Io.await))
                |> Io.andThen (\result -> urls result.deletedURLs |> Io.map (\deleted -> E.object [ ( "deletedURLs", deleted ) ]))
                |> unchanged

        "getCachedURLs" ->
            controller "controller" |> Io.andThen Precaching.getURLsToCacheKeys |> Io.andThen (List.map Tuple.first >> urls) |> unchanged

        "getURLsToCacheKeys" ->
            controller "controller"
                |> Io.andThen Precaching.getURLsToCacheKeys
                |> Io.andThen (Io.sequence << List.map (\( url, key ) -> relative url |> Io.andThen (\u -> relative key |> Io.map (\k -> E.list E.string [ u, k ]))))
                |> Io.map (E.list identity)
                |> unchanged

        "getCacheKeyForURL" ->
            controller "controller"
                |> Io.andThen (\found -> Precaching.getCacheKeyForURL found (required "url" value))
                |> Io.andThen (Maybe.map (relative >> Io.map E.string) >> Maybe.withDefault (Io.succeed E.null))
                |> unchanged

        "matchPrecache" ->
            controller "controller"
                |> Io.andThen
                    (\found ->
                        (if field "request" value |> Maybe.map Precaching.isTruthy |> Maybe.withDefault False then
                            Host.newRequest (required "url" value) Host.requestInit |> Io.map .url

                         else
                            Io.succeed (required "url" value)
                        )
                            |> Io.andThen (Precaching.matchPrecache found)
                    )
                |> Io.andThen (Maybe.map record >> Maybe.withDefault (Io.succeed E.null))
                |> unchanged

        "router" ->
            Routing.newRouter |> Io.map (\created -> ( { scenario | routers = Dict.insert (required "id" value) created scenario.routers }, E.null ))

        "precacheRoute" ->
            controller "controller"
                |> Io.andThen (\found -> routeOptions (field "options" value) |> Io.andThen (Precaching.precacheRoute found))
                |> Io.andThen (register scenario value)

        "route" ->
            let
                match options =
                    if field "crossOrigin" value |> Maybe.map Precaching.isTruthy |> Maybe.withDefault False then
                        Io.succeed (matched (not options.sameOrigin))

                    else
                        relative options.url.href |> Io.map (String.startsWith (required "prefix" value) >> matched)
            in
            handler scenario (field "handler" value)
                |> Io.andThen (\routeHandler -> Routing.newRoute match routeHandler method)
                |> Io.andThen (register scenario value)

        "defaultHandler" ->
            router |> Io.andThen (\found -> handler scenario (field "handler" value) |> Io.andThen (\routeHandler -> Routing.setDefaultHandler found routeHandler method)) |> done

        "catchHandler" ->
            (case text "route" value of
                Just routeId ->
                    lookup routeId scenario.routes |> Io.andThen (\route -> handler scenario (field "handler" value) |> Io.andThen (Routing.setRouteCatchHandler route))

                Nothing ->
                    router |> Io.andThen (\found -> handler scenario (field "handler" value) |> Io.andThen (Routing.setCatchHandler found))
            )
                |> done

        "unregisterRoute" ->
            router |> Io.andThen (\found -> lookup (required "route" value) scenario.routes |> Io.andThen (Routing.unregisterRoute found)) |> done

        "fetch" ->
            (case ( text "url" value, scenario.start.request ) of
                ( Just url, _ ) ->
                    let
                        init =
                            Host.requestInit
                    in
                    Host.newRequest url { init | method = text "method" value, mode = text "mode" value, integrity = text "integrity" value }

                ( Nothing, Just native ) ->
                    Io.succeed native

                ( Nothing, Nothing ) ->
                    Io.exit (Err "a fetch step without a url outside service-worker mode")
            )
                |> Io.andThen
                    (\request ->
                        router
                            |> Io.andThen
                                (\found ->
                                    lifetime scenario.start
                                        "fetch"
                                        (\event ->
                                            Routing.handleRequest found request event
                                                |> Io.andThen (Maybe.map (Io.await >> Io.map Just) >> Maybe.withDefault (Io.succeed Nothing))
                                        )
                                )
                    )
                |> Io.andThen (Maybe.map record >> Maybe.withDefault (Io.succeed E.null))
                |> unchanged

        "caches" ->
            dump |> unchanged

        op ->
            Io.exit (Err ("unknown step " ++ op))


{-| A cache name of the script, under the scenario's prefix.
-}
cacheName : String -> Wb String
cacheName name =
    Io.get (\world -> world.fixture.prefix ++ "-" ++ name)


matched : Bool -> Maybe (Maybe World.Params)
matched yes =
    if yes then
        Just Nothing

    else
        Nothing


register : Scenario -> D.Value -> Route -> Wb ( Scenario, E.Value )
register scenario value route =
    lookup (required "router" value) scenario.routers
        |> Io.andThen (\found -> Routing.registerRoute found route)
        |> Io.map
            (\() ->
                case text "id" value of
                    Just id ->
                        ( { scenario | routes = Dict.insert id route scenario.routes }, E.null )

                    Nothing ->
                        ( scenario, E.null )
            )


urls : List String -> Wb E.Value
urls values =
    Io.sequence (List.map relative values) |> Io.map (E.list E.string)


{-| A manifest entry from the corpus: falsy, a string, or a `PrecacheEntry` object.
-}
precacheEntry : D.Value -> PrecacheEntry
precacheEntry value =
    if not (Precaching.isTruthy value) then
        FalsyEntry value

    else
        case D.decodeValue D.string value of
            Ok url ->
                UrlEntry url

            Err _ ->
                ObjectEntry { url = text "url" value, revision = field "revision" value, integrity = text "integrity" value, source = value }



-- Handlers and route options


{-| A route handler: a strategy, a controller's precache strategy, a handler bound to a precached
URL, or an async function answering a fixed response.
-}
handler : Scenario -> Maybe D.Value -> Wb RouteHandler
handler scenario spec =
    let
        value =
            Maybe.withDefault E.null spec
    in
    case ( text "strategy" value, text "precache" value, text "bound" value ) of
        ( Just name, _, _ ) ->
            cacheName (required "cacheName" value)
                |> Io.andThen
                    (\strategyCacheName ->
                        plugins scenario (list "plugins" value)
                            |> Io.andThen
                                (\specs ->
                                    case name of
                                        "CacheFirst" ->
                                            Strategies.cacheFirst strategyCacheName specs

                                        "CacheOnly" ->
                                            Strategies.cacheOnly strategyCacheName specs

                                        "NetworkFirst" ->
                                            Strategies.networkFirst strategyCacheName specs

                                        "NetworkOnly" ->
                                            Strategies.networkOnly strategyCacheName specs

                                        _ ->
                                            Io.exit (Err ("unknown strategy " ++ name))
                                )
                    )
                |> Io.map (\strategy -> RouteHandler (Strategies.handle strategy))

        ( Nothing, Just id, _ ) ->
            lookup id scenario.controllers |> Io.map (\found -> RouteHandler (Strategies.handle found.strategy))

        ( Nothing, Nothing, Just url ) ->
            lookup (required "controller" value) scenario.controllers |> Io.andThen (\found -> Precaching.createHandlerBoundToURL found url)

        ( Nothing, Nothing, Nothing ) ->
            Io.succeed
                (RouteHandler
                    (\_ ->
                        Io.spawn Host.encodeResponse
                            Host.responseDecoder
                            (Host.newResponse (field "respond" value |> Maybe.withDefault E.null) (field "status" value |> Maybe.withDefault E.null))
                    )
                )


{-| PrecacheRoute options as given: an absent member keeps the default, `null` disables
`directoryIndex` and `cleanURLs`.
-}
routeOptions : Maybe D.Value -> Wb Precaching.RouteOptions
routeOptions given =
    Precaching.defaultRouteOptions
        |> Io.andThen
            (\defaults ->
                case given of
                    Nothing ->
                        Io.succeed defaults

                    Just options ->
                        (case field "ignoreURLParametersMatching" options of
                            Just sources ->
                                D.decodeValue (D.list D.string) sources |> Result.withDefault [] |> List.map Host.newRegExp |> Io.sequence

                            Nothing ->
                                Io.succeed defaults.ignoreURLParametersMatching
                        )
                            |> Io.map
                                (\ignoreURLParametersMatching ->
                                    { directoryIndex =
                                        case field "directoryIndex" options of
                                            Just index ->
                                                D.decodeValue D.string index |> Result.toMaybe

                                            Nothing ->
                                                defaults.directoryIndex
                                    , ignoreURLParametersMatching = ignoreURLParametersMatching
                                    , cleanURLs = field "cleanURLs" options |> Maybe.map Precaching.isTruthy |> Maybe.withDefault defaults.cleanURLs
                                    , urlManipulation =
                                        field "urlManipulation" options
                                            |> Maybe.map
                                                (\suffixes url ->
                                                    D.decodeValue (D.list D.string) suffixes
                                                        |> Result.withDefault []
                                                        |> List.map (\suffix -> Host.newUrl url.href Nothing |> Io.andThen (\variant -> Host.setPathname variant (variant.pathname ++ suffix)))
                                                        |> Io.sequence
                                                )
                                    }
                                )
            )



-- Events


{-| Runs one event-driven step under a fresh stand-in event and waits for every lifetime it was
given, in rounds, or, in service-worker mode, under the caller's native event.
-}
lifetime : Start -> String -> (Event -> Wb a) -> Wb a
lifetime start kind action =
    if start.native then
        Host.eventType |> Io.andThen (\nativeKind -> action (Event { kind = nativeKind, waitUntil = nativeWaitUntil }))

    else
        World.newId
            |> Io.andThen
                (\eventId ->
                    Io.modify (\world -> ( { world | standInEvents = Dict.insert eventId [] world.standInEvents }, () ))
                        |> Io.andThen
                            (\() ->
                                action (Event { kind = kind, waitUntil = \promise -> Io.modify (\world -> ( { world | standInEvents = Dict.update eventId (Maybe.map (\pending -> pending ++ [ promise ])) world.standInEvents }, () )) })
                                    |> Io.finally
                                        (settleLifetimes eventId 0
                                            |> Io.andThen (\count -> Io.modify (\world -> ( { world | fixture = (\fixture -> { fixture | events = E.list identity [ E.string kind, E.int count ] :: fixture.events }) world.fixture }, () )))
                                        )
                            )
                )


{-| The stand-in's lifetimes, awaited in rounds until no new ones arrive (`Promise.allSettled`):
their number.
-}
settleLifetimes : Int -> Int -> Wb Int
settleLifetimes eventId count =
    Io.modify
        (\world ->
            ( { world | standInEvents = Dict.insert eventId [] world.standInEvents }
            , Dict.get eventId world.standInEvents |> Maybe.withDefault []
            )
        )
        |> Io.andThen
            (\batch ->
                if List.isEmpty batch then
                    Io.succeed count

                else
                    Io.forEach (\promise -> Io.Await promise (\_ -> Io.succeed ())) batch
                        |> Io.andThen (\() -> settleLifetimes eventId (count + List.length batch))
            )


{-| `event.waitUntil(promise)` on the native event: the host's lifetime settles as the promise does.
-}
nativeWaitUntil : Int -> Wb ()
nativeWaitUntil promise =
    Host.eventWaitUntil
        |> Io.andThen
            (\token ->
                Io.spawn Strategies.encodeUnit
                    Strategies.unitDecoder
                    (Io.Await promise Io.Done
                        |> Io.andThen
                            (\outcome ->
                                Host.settleLifetime token
                                    (case outcome of
                                        Ok _ ->
                                            Nothing

                                        Err thrown ->
                                            Just ( Io.nameOf thrown, "" )
                                    )
                            )
                    )
                    |> Io.map (\_ -> ())
            )



-- Fixture plugins


{-| The network fixture, then the plugins the script names.
-}
plugins : Scenario -> List D.Value -> Wb (List Plugin)
plugins scenario specs =
    World.newId |> Io.andThen (\id -> Io.sequence (List.map (plugin scenario) specs) |> Io.map ((::) (networkPlugin id)))


plugin : Scenario -> D.Value -> Wb Plugin
plugin scenario spec =
    World.newId
        |> Io.andThen
            (\id ->
                let
                    callbacks =
                        World.noCallbacks id
                in
                case required "kind" spec of
                    "trace" ->
                        Io.succeed (tracePlugin id (required "label" spec))

                    "keep" ->
                        Io.succeed (Plugin { callbacks | cacheWillUpdate = Just (\param -> Io.succeed (Just param.response)) })

                    "drop" ->
                        Io.succeed (Plugin { callbacks | cacheWillUpdate = Just (\_ -> Io.succeed Nothing) })

                    "fail" ->
                        Io.succeed (failPlugin id (required "on" spec) (field "value" spec))

                    "fallback" ->
                        lookup (required "controller" spec) scenario.controllers |> Io.map (Precaching.precacheFallbackPlugin id (required "url" spec))

                    kind ->
                        Io.exit (Err ("unknown plugin " ++ kind))
            )


{-| The network fixture: every fetch goes to one of the two fixture files, or fails.
-}
networkPlugin : Int -> Plugin
networkPlugin id =
    let
        callbacks =
            World.noCallbacks id

        source =
            Io.get (.fixture >> .source)
    in
    Plugin
        { callbacks
            | requestWillFetch =
                Just
                    (\param ->
                        source
                            |> Io.andThen
                                (\current ->
                                    if current == "offline" then
                                        Io.throw (HostError { name = "TypeError", message = "offline", isError = True, text = "TypeError: offline" })

                                    else
                                        Io.get (.location >> .href)
                                            |> Io.andThen (\href -> Host.newUrl (fixturePath ++ (if current == "beta" then "beta" else "alpha") ++ ".mjs") (Just href))
                                            |> Io.andThen
                                                (\target ->
                                                    let
                                                        init =
                                                            Host.requestInit
                                                    in
                                                    Host.newRequest target.href { init | integrity = Just (if current == "broken" then wrongIntegrity else param.request.integrity) }
                                                )
                                )
                    )
            , fetchDidSucceed =
                Just
                    (\param ->
                        source
                            |> Io.andThen
                                (\current ->
                                    case current of
                                        "missing" ->
                                            Host.newResponse (E.string "missing") (E.int 404)

                                        "redirected" ->
                                            Host.markRedirected param.response

                                        _ ->
                                            Io.succeed param.response
                                )
                    )
        }


{-| The trace plugin: every lifecycle callback but cacheWillUpdate, recorded per handler. A
callback records only into a log its own handlerWillStart created: a plugin before it can fail
that callback.
-}
tracePlugin : Int -> String -> Plugin
tracePlugin id label =
    let
        callbacks =
            World.noCallbacks id

        log : Maybe StateRef -> Wb (List E.Value) -> Wb ()
        log state entryOf =
            case state of
                Nothing ->
                    Io.succeed ()

                Just ref ->
                    Strategies.getState ref "log"
                        |> Io.andThen
                            (\stored ->
                                case stored |> Maybe.andThen (D.decodeValue D.int >> Result.toMaybe) of
                                    Just logId ->
                                        entryOf
                                            |> Io.andThen
                                                (\fields ->
                                                    Io.modify
                                                        (\world ->
                                                            ( { world | fixture = (\fixture -> { fixture | logs = Dict.update logId (Maybe.map ((::) (E.list identity fields))) fixture.logs }) world.fixture }
                                                            , ()
                                                            )
                                                        )
                                                )

                                    Nothing ->
                                        Io.succeed ()
                            )

        url request =
            relative request.url |> Io.map E.string

        labelOf name =
            Io.get (\world -> E.string (cacheLabel world.fixture.prefix name))

        values =
            Io.sequence

        errorName =
            Io.nameOf >> E.string
    in
    Plugin
        { callbacks
            | handlerWillStart =
                Just
                    (\param ->
                        World.newId
                            |> Io.andThen
                                (\logId ->
                                    relative param.request.url
                                        |> Io.andThen
                                            (\requestUrl ->
                                                Io.modify
                                                    (\world ->
                                                        ( { world | fixture = (\fixture -> { fixture | traces = { log = logId, label = label, url = requestUrl } :: fixture.traces, logs = Dict.insert logId [] fixture.logs }) world.fixture }
                                                        , ()
                                                        )
                                                    )
                                            )
                                        |> Io.andThen (\() -> param.state |> Maybe.map (\ref -> Strategies.setState ref "log" (E.int logId)) |> Maybe.withDefault (Io.succeed ()))
                                )
                    )
            , cacheKeyWillBeUsed =
                Just (\param -> log param.state (values [ Io.succeed (E.string "cacheKeyWillBeUsed"), Io.succeed (E.string param.mode), url param.request ]) |> Io.map (\() -> param.request))
            , cachedResponseWillBeUsed =
                Just
                    (\param ->
                        log param.state (values [ Io.succeed (E.string "cachedResponseWillBeUsed"), labelOf param.cacheName, url param.request, Io.succeed (status param.cachedResponse) ])
                            |> Io.map (\() -> param.cachedResponse)
                    )
            , requestWillFetch =
                Just (\param -> log param.state (values [ Io.succeed (E.string "requestWillFetch"), url param.request ]) |> Io.map (\() -> param.request))
            , fetchDidSucceed =
                Just (\param -> log param.state (values [ Io.succeed (E.string "fetchDidSucceed"), url param.request, Io.succeed (E.int param.response.status) ]) |> Io.map (\() -> param.response))
            , fetchDidFail =
                Just (\param -> log param.state (values [ Io.succeed (E.string "fetchDidFail"), url param.originalRequest, url param.request, Io.succeed (errorName param.error) ]))
            , cacheDidUpdate =
                Just
                    (\param ->
                        log param.state
                            (values [ Io.succeed (E.string "cacheDidUpdate"), labelOf param.cacheName, url param.request, Io.succeed (status param.oldResponse), Io.succeed (E.int param.newResponse.status) ])
                    )
            , handlerWillRespond =
                Just (\param -> log param.state (values [ Io.succeed (E.string "handlerWillRespond"), Io.succeed (E.int param.response.status) ]) |> Io.map (\() -> param.response))
            , handlerDidRespond =
                Just (\param -> log param.state (values [ Io.succeed (E.string "handlerDidRespond"), Io.succeed (status param.response) ]))
            , handlerDidComplete =
                Just
                    (\param ->
                        log param.state (values [ Io.succeed (E.string "handlerDidComplete"), Io.succeed (status param.response), Io.succeed (param.error |> Maybe.map errorName |> Maybe.withDefault E.null) ])
                    )
            , handlerDidError =
                Just (\param -> log param.state (values [ Io.succeed (E.string "handlerDidError"), Io.succeed (errorName param.error) ]) |> Io.map (\() -> Nothing))
        }


{-| A plugin whose one callback, the one named `on`, throws `new Error('fail')`, or, with a value,
that value itself (a non-Error).
-}
failPlugin : Int -> String -> Maybe D.Value -> Plugin
failPlugin id on value =
    let
        callbacks =
            World.noCallbacks id

        thrown =
            case value of
                Nothing ->
                    HostError { name = "Error", message = "fail", isError = True, text = "Error: fail" }

                Just given ->
                    HostError { name = "", message = "", isError = False, text = D.decodeValue D.string given |> Result.withDefault (E.encode 0 given) }

        failing name =
            if name == on then
                Just (\_ -> Io.throw thrown)

            else
                Nothing
    in
    Plugin
        { callbacks
            | handlerWillStart = failing "handlerWillStart"
            , cacheKeyWillBeUsed = failing "cacheKeyWillBeUsed"
            , cachedResponseWillBeUsed = failing "cachedResponseWillBeUsed"
            , cacheWillUpdate = failing "cacheWillUpdate"
            , cacheDidUpdate = failing "cacheDidUpdate"
            , requestWillFetch = failing "requestWillFetch"
            , fetchDidSucceed = failing "fetchDidSucceed"
            , fetchDidFail = failing "fetchDidFail"
            , handlerWillRespond = failing "handlerWillRespond"
            , handlerDidRespond = failing "handlerDidRespond"
            , handlerDidComplete = failing "handlerDidComplete"
            , handlerDidError = failing "handlerDidError"
        }
