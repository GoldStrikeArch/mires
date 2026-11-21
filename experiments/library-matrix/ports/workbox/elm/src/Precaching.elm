module Precaching exposing
    ( PrecacheEntry(..), InstallResult, CleanupResult, RouteOptions, defaultRouteOptions
    , newPrecacheController, addToCacheList, install, activate
    , getURLsToCacheKeys, getCacheKeyForURL, matchPrecache, createHandlerBoundToURL
    , precacheRoute, precacheFallbackPlugin
    , isTruthy
    )

{-| workbox-precaching: PrecacheController, PrecacheStrategy, PrecacheRoute with its URL
variations, createCacheKey, and the cache-key, install-report, fallback, default-cacheability and
redirect-copy plugins.
-}

import Dict
import Host exposing (RegExp, Request, Response, Url)
import Io exposing (Promise, Thrown(..))
import Json.Decode as D
import Json.Encode as E
import Routing
import Strategies
import World exposing (ControllerState, Event, Params, Plugin(..), PrecacheController, Route, RouteHandler(..), Strategy, StrategyHandler, Wb)
import WorkboxCore exposing (copyResponse, workboxError)


revisionSearchParam : String
revisionSearchParam =
    "__WB_REVISION__"


{-| A precache manifest entry as the caller wrote it (`PrecacheEntry | string`). An object's
`revision` stays the caller's JSON value (Nothing where it is absent, which is undefined): its
truthiness decides, and the host prints a number. `source` is the object itself, which
add-to-cache-list-unexpected-type reports.
-}
type PrecacheEntry
    = UrlEntry String
    | ObjectEntry { url : Maybe String, revision : Maybe D.Value, integrity : Maybe String, source : D.Value }
    | FalsyEntry D.Value


{-| A string that is truthy: present and not empty.
-}
nonEmpty : Maybe String -> Maybe String
nonEmpty =
    Maybe.andThen
        (\value ->
            if String.isEmpty value then
                Nothing

            else
                Just value
        )


{-| JavaScript truthiness of a JSON value.
-}
isTruthy : D.Value -> Bool
isTruthy value =
    D.decodeValue
        (D.oneOf
            [ D.null False
            , D.map (not << String.isEmpty) D.string
            , D.map (\number -> number /= 0 && not (isNaN number)) D.float
            , D.bool
            , D.succeed True
            ]
        )
        value
        |> Result.withDefault True


type alias InstallResult =
    { updatedURLs : List String
    , notUpdatedURLs : List String
    }


type alias CleanupResult =
    { deletedURLs : List String
    }


unexpectedType : D.Value -> Thrown
unexpectedType entry =
    workboxError "add-to-cache-list-unexpected-type" [ ( "entry", entry ) ]


{-| `createCacheKey(entry)`: the URL resolved against the location, and with a truthy revision a
cache key that carries it in `__WB_REVISION__`. Answers `(cacheKey, url)`.
-}
createCacheKey : String -> PrecacheEntry -> Wb ( String, String )
createCacheKey base entry =
    let
        own url =
            Host.newUrl url (Just base) |> Io.map (\resolved -> ( resolved.href, resolved.href ))
    in
    case entry of
        FalsyEntry source ->
            Io.throw (unexpectedType source)

        UrlEntry url ->
            own url

        ObjectEntry object ->
            case object.url of
                Just url ->
                    if String.isEmpty url then
                        Io.throw (unexpectedType object.source)

                    else
                        case object.revision |> Maybe.andThen (\revision -> if isTruthy revision then Just revision else Nothing) of
                            Nothing ->
                                own url

                            Just revision ->
                                Host.newUrl url (Just base)
                                    |> Io.andThen
                                        (\cacheKeyURL ->
                                            Host.newUrl url (Just base)
                                                |> Io.andThen
                                                    (\originalURL ->
                                                        Host.searchParamSet cacheKeyURL revisionSearchParam revision
                                                            |> Io.map (\withRevision -> ( withRevision.href, originalURL.href ))
                                                    )
                                        )

                Nothing ->
                    Io.throw (unexpectedType object.source)



-- PrecacheController


controllerState : PrecacheController -> World.World -> ControllerState
controllerState controller world =
    Dict.get controller.id world.controllers
        |> Maybe.withDefault { urls = [], urlsToCacheKeys = Dict.empty, urlsToCacheModes = Dict.empty, cacheKeysToIntegrities = Dict.empty }


modifyController : PrecacheController -> (ControllerState -> ControllerState) -> Wb ()
modifyController controller change =
    Io.modify (\world -> ( { world | controllers = Dict.insert controller.id (change (controllerState controller world)) world.controllers }, () ))


{-| `new PrecacheController({cacheName, plugins, fallbackToNetwork})`: a PrecacheStrategy whose
plugins are the caller's, then the controller's cache-key plugin, then the redirect copy.
-}
newPrecacheController : String -> List Plugin -> Bool -> Wb PrecacheController
newPrecacheController cacheName plugins fallbackToNetwork =
    World.newId
        |> Io.andThen
            (\id ->
                World.newId
                    |> Io.andThen
                        (\pluginId ->
                            newPrecacheStrategy cacheName (plugins ++ [ precacheCacheKeyPlugin pluginId id ]) fallbackToNetwork
                                |> Io.andThen
                                    (\strategy ->
                                        let
                                            controller =
                                                { id = id, strategy = strategy }
                                        in
                                        modifyController controller identity |> Io.map (\() -> controller)
                                    )
                        )
            )


{-| `addToCacheList(entries)`: entry by entry, so the entries before a failing one stay added. Like
the production build it warns about revisionless entries.
-}
addToCacheList : PrecacheController -> List PrecacheEntry -> Wb ()
addToCacheList controller entries =
    let
        add entry urlsToWarnAbout =
            let
                warnings =
                    case entry of
                        UrlEntry url ->
                            urlsToWarnAbout ++ [ url ]

                        ObjectEntry object ->
                            case object.revision of
                                Nothing ->
                                    urlsToWarnAbout ++ [ Maybe.withDefault "" object.url ]

                                Just _ ->
                                    urlsToWarnAbout

                        FalsyEntry _ ->
                            urlsToWarnAbout

                ( cacheMode, entryIntegrity ) =
                    case entry of
                        ObjectEntry object ->
                            ( if object.revision |> Maybe.map isTruthy |> Maybe.withDefault False then "reload" else "default"
                            , nonEmpty object.integrity
                            )

                        _ ->
                            ( "default", Nothing )
            in
            Io.get (.location >> .href)
                |> Io.andThen (\base -> createCacheKey base entry)
                |> Io.andThen
                    (\( cacheKey, url ) ->
                        Io.get (controllerState controller)
                            |> Io.andThen
                                (\state ->
                                    case Dict.get url state.urlsToCacheKeys of
                                        Just firstEntry ->
                                            if firstEntry /= cacheKey then
                                                Io.throw (workboxError "add-to-cache-list-conflicting-entries" [ ( "firstEntry", E.string firstEntry ), ( "secondEntry", E.string cacheKey ) ])

                                            else
                                                record state cacheKey url cacheMode entryIntegrity

                                        Nothing ->
                                            record state cacheKey url cacheMode entryIntegrity
                                )
                    )
                |> Io.andThen (\() -> if List.isEmpty warnings then Io.succeed () else Host.warn (warningMessage warnings))
                |> Io.map (\() -> warnings)

        record state cacheKey url cacheMode entryIntegrity =
            case entryIntegrity |> Maybe.andThen (\integrity -> Dict.get cacheKey state.cacheKeysToIntegrities |> Maybe.map (Tuple.pair integrity)) of
                Just ( integrity, known ) ->
                    if known /= integrity then
                        Io.throw (workboxError "add-to-cache-list-conflicting-integrities" [ ( "url", E.string url ) ])

                    else
                        set cacheKey url cacheMode entryIntegrity

                Nothing ->
                    set cacheKey url cacheMode entryIntegrity

        set cacheKey url cacheMode entryIntegrity =
            modifyController controller
                (\state ->
                    { state
                        | urls = if Dict.member url state.urlsToCacheKeys then state.urls else state.urls ++ [ url ]
                        , urlsToCacheKeys = Dict.insert url cacheKey state.urlsToCacheKeys
                        , urlsToCacheModes = Dict.insert url cacheMode state.urlsToCacheModes
                        , cacheKeysToIntegrities = entryIntegrity |> Maybe.map (\integrity -> Dict.insert cacheKey integrity state.cacheKeysToIntegrities) |> Maybe.withDefault state.cacheKeysToIntegrities
                    }
                )
    in
    Io.foldl add [] entries |> Io.map (\_ -> ())


warningMessage : List String -> String
warningMessage urls =
    "Workbox is precaching URLs without revision info: " ++ String.join ", " urls ++ "\nThis is generally NOT safe. Learn more at https://bit.ly/wb-precache"


{-| `getURLsToCacheKeys()`, in insertion order.
-}
getURLsToCacheKeys : PrecacheController -> Wb (List ( String, String ))
getURLsToCacheKeys controller =
    Io.get (controllerState controller >> entriesOf)


entriesOf : ControllerState -> List ( String, String )
entriesOf state =
    List.filterMap (\url -> Dict.get url state.urlsToCacheKeys |> Maybe.map (Tuple.pair url)) state.urls


{-| `getCacheKeyForURL(url)`: the exact URL resolved against the location, without variations.
-}
getCacheKeyForURL : PrecacheController -> String -> Wb (Maybe String)
getCacheKeyForURL controller url =
    Io.get (.location >> .href)
        |> Io.andThen (\base -> Host.newUrl url (Just base))
        |> Io.andThen (\resolved -> Io.get (controllerState controller >> .urlsToCacheKeys >> Dict.get resolved.href))


{-| `install(event)`: precaches each URL in turn through the strategy, under an install report
plugin, and extends the event's lifetime until it is done.
-}
install : PrecacheController -> Event -> Wb (Promise InstallResult)
install controller event =
    WorkboxCore.waitUntil event
        (\result -> E.object [ ( "updatedURLs", E.list E.string result.updatedURLs ), ( "notUpdatedURLs", E.list E.string result.notUpdatedURLs ) ])
        (D.map2 InstallResult (D.field "updatedURLs" (D.list D.string)) (D.field "notUpdatedURLs" (D.list D.string)))
        (World.newId
            |> Io.andThen
                (\reportId ->
                    Io.modify (\world -> ( { world | reports = Dict.insert reportId { updatedURLs = [], notUpdatedURLs = [] } world.reports }, () ))
                        |> Io.andThen (\() -> Strategies.modifyPlugins controller.strategy (\plugins -> plugins ++ [ precacheInstallReportPlugin reportId ]))
                        -- Steps run one at a time, so the map cannot change during an install.
                        |> Io.andThen (\() -> Io.get (controllerState controller))
                        |> Io.andThen (\state -> Io.forEach (precacheEntry controller event state) (entriesOf state))
                        |> Io.andThen (\() -> Io.get (.reports >> Dict.get reportId))
                        |> Io.map
                            (\report ->
                                { updatedURLs = report |> Maybe.map (.updatedURLs >> List.reverse) |> Maybe.withDefault []
                                , notUpdatedURLs = report |> Maybe.map (.notUpdatedURLs >> List.reverse) |> Maybe.withDefault []
                                }
                            )
                )
        )


{-| One URL of an install: `await Promise.all(strategy.handleAll({params: {cacheKey}, request, event}))`.
-}
precacheEntry : PrecacheController -> Event -> ControllerState -> ( String, String ) -> Wb ()
precacheEntry controller event state ( url, cacheKey ) =
    let
        init =
            Host.requestInit
    in
    Host.newRequest url { init | integrity = Dict.get cacheKey state.cacheKeysToIntegrities, cache = Dict.get url state.urlsToCacheModes, credentials = Just "same-origin" }
        |> Io.andThen (\request -> Strategies.handleAll controller.strategy { request = request, event = event, params = Just { cacheKey = Just cacheKey, integrity = Nothing } })
        -- Promise.all: the response's failure, else the completion's (which comes later).
        |> Io.andThen (\( responseDone, handlerDone ) -> Io.await responseDone |> Io.andThen (\_ -> Io.await handlerDone))


{-| `activate(event)`: deletes every request in the precache whose URL is no current cache key.
-}
activate : PrecacheController -> Event -> Wb (Promise CleanupResult)
activate controller event =
    WorkboxCore.waitUntil event
        (\result -> E.object [ ( "deletedURLs", E.list E.string result.deletedURLs ) ])
        (D.map CleanupResult (D.field "deletedURLs" (D.list D.string)))
        (Host.cachesOpen (World.strategyCacheName controller.strategy)
            |> Io.andThen
                (\cache ->
                    Host.cacheKeys cache
                        |> Io.andThen
                            (\currentlyCachedRequests ->
                                Io.get (controllerState controller >> .urlsToCacheKeys >> Dict.values)
                                    |> Io.andThen
                                        (\expectedCacheKeys ->
                                            Io.foldl
                                                (\request deletedURLs ->
                                                    if List.member request.url expectedCacheKeys then
                                                        Io.succeed deletedURLs

                                                    else
                                                        Host.cacheDelete cache request |> Io.map (\_ -> deletedURLs ++ [ request.url ])
                                                )
                                                []
                                                currentlyCachedRequests
                                        )
                            )
                )
            |> Io.map CleanupResult
        )


{-| `matchPrecache(request)`: opens the precache only for a URL in the manifest.
-}
matchPrecache : PrecacheController -> String -> Wb (Maybe Response)
matchPrecache controller url =
    getCacheKeyForURL controller url
        |> Io.andThen
            (\found ->
                case found of
                    Just cacheKey ->
                        Host.cachesOpen (World.strategyCacheName controller.strategy) |> Io.andThen (\cache -> Host.cacheMatchUrl cache cacheKey)

                    Nothing ->
                        Io.succeed Nothing
            )


{-| `createHandlerBoundToURL(url)`: a handler that answers any request with this precached URL.
-}
createHandlerBoundToURL : PrecacheController -> String -> Wb RouteHandler
createHandlerBoundToURL controller url =
    getCacheKeyForURL controller url
        |> Io.andThen
            (\found ->
                case found of
                    Nothing ->
                        Io.throw (workboxError "non-precached-url" [ ( "url", E.string url ) ])

                    Just cacheKey ->
                        Io.succeed
                            (RouteHandler
                                (\options ->
                                    let
                                        -- `{cacheKey, ...options.params}`.
                                        params =
                                            { cacheKey = options.params |> Maybe.andThen .cacheKey |> Maybe.withDefault cacheKey |> Just
                                            , integrity = options.params |> Maybe.andThen .integrity
                                            }
                                    in
                                    Host.newRequest url Host.requestInit
                                        |> Io.andThen (\request -> Strategies.handle controller.strategy { request = request, event = options.event, params = Just params })
                                )
                            )
            )



-- PrecacheStrategy


{-| `new PrecacheStrategy({cacheName, plugins, fallbackToNetwork})`, which appends the redirect
copy plugin.
-}
newPrecacheStrategy : String -> List Plugin -> Bool -> Wb Strategy
newPrecacheStrategy cacheName plugins fallbackToNetwork =
    Strategies.newStrategy cacheName (plugins ++ [ copyRedirectedCacheableResponsesPlugin ]) (precacheHandle cacheName fallbackToNetwork)


{-| PrecacheStrategy's `_handle`: the cache, then install-time population, then the network.
-}
precacheHandle : String -> Bool -> Request -> StrategyHandler -> Wb Response
precacheHandle cacheName fallbackToNetwork request handler =
    Strategies.cacheMatch handler request
        |> Io.andThen
            (\cached ->
                case cached of
                    Just response ->
                        Io.succeed response

                    Nothing ->
                        if World.eventKind handler.event == "install" then
                            handleInstall request handler

                        else
                            handleFetch cacheName fallbackToNetwork request handler
            )


{-| `_handleFetch`: the network, with the manifest's integrity unless the request is no-cors, and
a cache repair only when that integrity vouches for the response.
-}
handleFetch : String -> Bool -> Request -> StrategyHandler -> Wb Response
handleFetch cacheName fallbackToNetwork request handler =
    let
        params =
            Maybe.withDefault World.noParams handler.params

        integrityInManifest =
            nonEmpty params.integrity

        integrityInRequest =
            request.integrity

        noIntegrityConflict =
            String.isEmpty integrityInRequest || Just integrityInRequest == integrityInManifest

        cors =
            request.mode /= "no-cors"

        init =
            Host.requestInit

        integrity =
            if not cors then
                Nothing

            else if String.isEmpty integrityInRequest then
                integrityInManifest

            else
                Just integrityInRequest
    in
    if not fallbackToNetwork then
        Io.throw (workboxError "missing-precache-entry" [ ( "cacheName", E.string cacheName ), ( "url", E.string request.url ) ])

    else
        Host.requestFromRequest request { init | integrity = integrity }
            |> Io.andThen (Strategies.fetch handler)
            |> Io.andThen
                (\response ->
                    if integrityInManifest /= Nothing && noIntegrityConflict && cors then
                        useDefaultCacheabilityPluginIfNeeded handler.strategy
                            |> Io.andThen (\() -> Host.cloneResponse response)
                            |> Io.andThen (Strategies.cachePut handler request)
                            |> Io.map (\_ -> response)

                    else
                        Io.succeed response
                )


{-| `_handleInstall`: fetch, then cache, and fail the install when the response may not be cached.
-}
handleInstall : Request -> StrategyHandler -> Wb Response
handleInstall request handler =
    useDefaultCacheabilityPluginIfNeeded handler.strategy
        |> Io.andThen (\() -> Strategies.fetch handler request)
        |> Io.andThen
            (\response ->
                Host.cloneResponse response
                    |> Io.andThen (Strategies.cachePut handler request)
                    |> Io.andThen
                        (\wasCached ->
                            if wasCached then
                                Io.succeed response

                            else
                                Io.throw (workboxError "bad-precaching-response" [ ( "url", E.string request.url ), ( "status", E.int response.status ) ])
                        )
            )


{-| `_useDefaultCacheabilityPluginIfNeeded`: exactly one cacheWillUpdate besides the redirect copy,
adding the default one when there is none and dropping it when there are more.
-}
useDefaultCacheabilityPluginIfNeeded : Strategy -> Wb ()
useDefaultCacheabilityPluginIfNeeded strategy =
    Strategies.modifyPlugins strategy
        (\plugins ->
            let
                counted =
                    List.indexedMap Tuple.pair plugins |> List.filter (\( _, plugin ) -> World.pluginId plugin /= copyRedirectedId)

                defaultPluginIndex =
                    counted |> List.filter (\( _, plugin ) -> World.pluginId plugin == defaultCacheabilityId) |> List.reverse |> List.head |> Maybe.map Tuple.first

                definesCacheWillUpdate ( _, Plugin callbacks ) =
                    case callbacks.cacheWillUpdate of
                        Just _ ->
                            True

                        Nothing ->
                            False

                cacheWillUpdatePluginCount =
                    counted |> List.filter definesCacheWillUpdate |> List.length
            in
            if cacheWillUpdatePluginCount == 0 then
                plugins ++ [ defaultPrecacheCacheabilityPlugin ]

            else
                case ( cacheWillUpdatePluginCount > 1, defaultPluginIndex ) of
                    ( True, Just index ) ->
                        List.take index plugins ++ List.drop (index + 1) plugins

                    _ ->
                        plugins
        )


defaultCacheabilityId : Int
defaultCacheabilityId =
    2


copyRedirectedId : Int
copyRedirectedId =
    3


{-| `PrecacheStrategy.defaultPrecacheCacheabilityPlugin`: statuses below 400.
-}
defaultPrecacheCacheabilityPlugin : Plugin
defaultPrecacheCacheabilityPlugin =
    let
        callbacks =
            World.noCallbacks defaultCacheabilityId
    in
    Plugin { callbacks | cacheWillUpdate = Just (\param -> Io.succeed (if param.response.status >= 400 then Nothing else Just param.response)) }


{-| `PrecacheStrategy.copyRedirectedCacheableResponsesPlugin`: a redirected response is cached as a
copy without its redirect.
-}
copyRedirectedCacheableResponsesPlugin : Plugin
copyRedirectedCacheableResponsesPlugin =
    let
        callbacks =
            World.noCallbacks copyRedirectedId
    in
    Plugin
        { callbacks
            | cacheWillUpdate =
                Just (\param -> (if param.response.redirected then copyResponse param.response else Io.succeed param.response) |> Io.map Just)
        }


{-| `PrecacheCacheKeyPlugin`: the route's cache key, else the manifest's key for the exact URL.
-}
precacheCacheKeyPlugin : Int -> Int -> Plugin
precacheCacheKeyPlugin id controllerId =
    let
        callbacks =
            World.noCallbacks id

        lookup url =
            Io.get (.location >> .href)
                |> Io.andThen (\base -> Host.newUrl url (Just base))
                |> Io.andThen (\resolved -> Io.get (.controllers >> Dict.get controllerId >> Maybe.andThen (.urlsToCacheKeys >> Dict.get resolved.href)))
    in
    Plugin
        { callbacks
            | cacheKeyWillBeUsed =
                Just
                    (\param ->
                        (case param.params |> Maybe.andThen .cacheKey |> nonEmpty of
                            Just cacheKey ->
                                Io.succeed (Just cacheKey)

                            Nothing ->
                                lookup param.request.url
                        )
                            |> Io.andThen
                                (\cacheKey ->
                                    case cacheKey of
                                        Just key ->
                                            Host.requestHeaders param.request
                                                |> Io.andThen
                                                    (\headers ->
                                                        let
                                                            init =
                                                                Host.requestInit
                                                        in
                                                        Host.newRequest key { init | headers = Just headers }
                                                    )

                                        Nothing ->
                                            Io.succeed param.request
                                )
                    )
        }


{-| `PrecacheInstallReportPlugin`: `handlerWillStart` keeps the original request in its state, and
an install-time cache hit is not updated, a miss is.
-}
precacheInstallReportPlugin : Int -> Plugin
precacheInstallReportPlugin id =
    let
        callbacks =
            World.noCallbacks id

        report cached url world =
            { world
                | reports =
                    Dict.update id
                        (Maybe.map
                            (\current ->
                                if cached then
                                    { current | notUpdatedURLs = url :: current.notUpdatedURLs }

                                else
                                    { current | updatedURLs = url :: current.updatedURLs }
                            )
                        )
                        world.reports
            }
    in
    Plugin
        { callbacks
            | handlerWillStart =
                Just
                    (\param ->
                        case param.state of
                            Just state ->
                                Strategies.setState state "originalRequest" (Host.encodeRequest param.request)

                            Nothing ->
                                Io.succeed ()
                    )
            , cachedResponseWillBeUsed =
                Just
                    (\param ->
                        (case ( World.eventKind param.event, param.state ) of
                            ( "install", Just state ) ->
                                Strategies.getState state "originalRequest"
                                    |> Io.andThen
                                        (\stored ->
                                            case stored |> Maybe.andThen (D.decodeValue Host.requestDecoder >> Result.toMaybe) of
                                                Just originalRequest ->
                                                    Io.modify (\world -> ( report (param.cachedResponse /= Nothing) originalRequest.url world, () ))

                                                Nothing ->
                                                    Io.succeed ()
                                        )

                            _ ->
                                Io.succeed ()
                        )
                            |> Io.map (\() -> param.cachedResponse)
                    )
        }


{-| `PrecacheFallbackPlugin({fallbackURL, precacheController})`: a failed strategy answers with a
precached URL, when it is there.
-}
precacheFallbackPlugin : Int -> String -> PrecacheController -> Plugin
precacheFallbackPlugin id fallbackURL precacheController =
    let
        callbacks =
            World.noCallbacks id
    in
    Plugin { callbacks | handlerDidError = Just (\_ -> matchPrecache precacheController fallbackURL) }



-- PrecacheRoute


{-| `PrecacheRouteOptions`. A falsy `directoryIndex` or `cleanURLs` disables it.
-}
type alias RouteOptions =
    { directoryIndex : Maybe String
    , ignoreURLParametersMatching : List RegExp
    , cleanURLs : Bool
    , urlManipulation : Maybe (Url -> Wb (List Url))
    }


{-| The defaults: `index.html`, `[/^utm_/, /^fbclid$/]`, clean URLs, no manipulation.
-}
defaultRouteOptions : Wb RouteOptions
defaultRouteOptions =
    Host.newRegExp "^utm_"
        |> Io.andThen
            (\utm ->
                Host.newRegExp "^fbclid$"
                    |> Io.map (\fbclid -> { directoryIndex = Just "index.html", ignoreURLParametersMatching = [ utm, fbclid ], cleanURLs = True, urlManipulation = Nothing })
            )


{-| `removeIgnoredSearchParams(urlObject, ignoreURLParametersMatching)`, on the URL itself.
-}
removeIgnoredSearchParams : Url -> List RegExp -> Wb Url
removeIgnoredSearchParams urlObject ignoreURLParametersMatching =
    let
        ignored paramName patterns =
            case patterns of
                [] ->
                    Io.succeed False

                pattern :: rest ->
                    Host.regExpTest pattern paramName |> Io.andThen (\matches -> if matches then Io.succeed True else ignored paramName rest)
    in
    Host.searchParamKeys urlObject
        |> Io.andThen
            (Io.foldl
                (\paramName current ->
                    ignored paramName ignoreURLParametersMatching
                        |> Io.andThen (\matches -> if matches then Host.searchParamDelete current paramName else Io.succeed current)
                )
                urlObject
            )


{-| `generateURLVariations(url, options)`, consumed as the route consumes it: each variation in
order until `found` accepts one. Later variations (and `urlManipulation`) are not computed.
-}
findVariation : String -> RouteOptions -> (String -> Maybe hit) -> Wb (Maybe hit)
findVariation url options found =
    let
        try candidate otherwise =
            case found candidate of
                Just hit ->
                    Io.succeed (Just hit)

                Nothing ->
                    otherwise ()

        tryEach candidates =
            case candidates of
                [] ->
                    Io.succeed Nothing

                candidate :: rest ->
                    try candidate.href (\() -> tryEach rest)

        appended urlWithoutIgnoredParams suffix =
            Host.newUrl urlWithoutIgnoredParams.href Nothing |> Io.andThen (\variant -> Host.setPathname variant (variant.pathname ++ suffix))
    in
    Io.get (.location >> .href)
        |> Io.andThen (\base -> Host.newUrl url (Just base))
        |> Io.andThen (\parsed -> Host.setHash parsed "")
        |> Io.andThen
            (\urlObject ->
                try urlObject.href <|
                    \() ->
                        -- The URL object itself loses its ignored parameters; urlManipulation sees it
                        -- that way.
                        removeIgnoredSearchParams urlObject options.ignoreURLParametersMatching
                            |> Io.andThen
                                (\urlWithoutIgnoredParams ->
                                    try urlWithoutIgnoredParams.href <|
                                        \() ->
                                            (case options.directoryIndex of
                                                Just directoryIndex ->
                                                    if not (String.isEmpty directoryIndex) && String.endsWith "/" urlWithoutIgnoredParams.pathname then
                                                        appended urlWithoutIgnoredParams directoryIndex |> Io.map List.singleton

                                                    else
                                                        Io.succeed []

                                                Nothing ->
                                                    Io.succeed []
                                            )
                                                |> Io.andThen
                                                    (\directoryURL ->
                                                        tryEach directoryURL
                                                            |> orElse
                                                                (\() ->
                                                                    (if options.cleanURLs then
                                                                        appended urlWithoutIgnoredParams ".html" |> Io.map List.singleton

                                                                     else
                                                                        Io.succeed []
                                                                    )
                                                                        |> Io.andThen tryEach
                                                                        |> orElse
                                                                            (\() ->
                                                                                case options.urlManipulation of
                                                                                    Just urlManipulation ->
                                                                                        urlManipulation urlWithoutIgnoredParams |> Io.andThen tryEach

                                                                                    Nothing ->
                                                                                        Io.succeed Nothing
                                                                            )
                                                                )
                                                    )
                                )
            )


orElse : (() -> Wb (Maybe hit)) -> Wb (Maybe hit) -> Wb (Maybe hit)
orElse next first =
    first
        |> Io.andThen
            (\hit ->
                case hit of
                    Just _ ->
                        Io.succeed hit

                    Nothing ->
                        next ()
            )


{-| `new PrecacheRoute(controller, options)`: a GET route to the controller's strategy that matches
the first variation of the request URL in the manifest, with `{cacheKey, integrity}`.
-}
precacheRoute : PrecacheController -> RouteOptions -> Wb Route
precacheRoute controller options =
    Routing.newRoute
        (\matchOptions ->
            Io.get (controllerState controller)
                |> Io.andThen
                    (\state ->
                        findVariation matchOptions.request.url options (\possibleURL -> Dict.get possibleURL state.urlsToCacheKeys)
                            |> Io.map
                                (Maybe.map
                                    (\cacheKey -> Just { cacheKey = Just cacheKey, integrity = Dict.get cacheKey state.cacheKeysToIntegrities })
                                )
                    )
        )
        (RouteHandler (Strategies.handle controller.strategy))
        "GET"
