module Strategies exposing
    ( newStrategy, modifyPlugins, handle, handleAll
    , fetch, fetchAndCachePut, cacheMatch, cachePut, hasCallback
    , getState, setState
    , cacheFirst, cacheOnly, networkFirst, networkOnly
    , encodeUnit, unitDecoder
    )

{-| workbox-strategies: Strategy with its handle/handleAll lifecycle, StrategyHandler, and the
CacheFirst, CacheOnly, NetworkFirst and NetworkOnly strategies.
-}

import Dict
import Host exposing (Request, Response)
import Io exposing (Io, Promise, Thrown(..))
import Json.Decode as D
import Json.Encode as E
import World exposing (Callbacks, Event, HandlerCallbackOptions, Plugin(..), StateRef, Strategy(..), StrategyHandler, Wb)
import WorkboxCore exposing (cacheMatchIgnoreParams, workboxError)


encodeUnit : () -> E.Value
encodeUnit () =
    E.null


unitDecoder : D.Decoder ()
unitDecoder =
    D.succeed ()


encodeMaybeResponse : Maybe Response -> E.Value
encodeMaybeResponse =
    Maybe.map Host.encodeResponse >> Maybe.withDefault E.null


isJust : Maybe a -> Bool
isJust maybe =
    case maybe of
        Just _ ->
            True

        Nothing ->
            False


{-| A strategy with its cache name (always supplied: `cacheNames.getRuntimeName(cacheName)`
returns it unchanged), its plugin list and its subclass's `_handle`.
-}
newStrategy : String -> List Plugin -> (Request -> StrategyHandler -> Wb Response) -> Wb Strategy
newStrategy cacheName plugins handleRequest =
    World.newId
        |> Io.andThen
            (\id ->
                Io.modify (\world -> ( { world | strategies = Dict.insert id plugins world.strategies }, () ))
                    |> Io.map (\() -> Strategy { id = id, cacheName = cacheName, handleRequest = handleRequest })
            )


pluginsOf : Strategy -> World.World -> List Plugin
pluginsOf strategy world =
    Dict.get (World.strategyId strategy) world.strategies |> Maybe.withDefault []


{-| `strategy.plugins` changed in place: `push`, `unshift` or `splice`.
-}
modifyPlugins : Strategy -> (List Plugin -> List Plugin) -> Wb ()
modifyPlugins strategy change =
    Io.modify (\world -> ( { world | strategies = Dict.update (World.strategyId strategy) (Maybe.map change) world.strategies }, () ))


{-| `hasCallback(name)`, over the strategy's current plugins.
-}
hasCallback : (Callbacks -> Maybe callback) -> Strategy -> Wb Bool
hasCallback select strategy =
    Io.get (pluginsOf strategy >> List.any (\(Plugin callbacks) -> isJust (select callbacks)))



-- The handler's plugin state


stateDict : StateRef -> World.World -> Dict.Dict String D.Value
stateDict ref world =
    Dict.get ref.handler world.handlers |> Maybe.andThen (.states >> Dict.get ref.plugin) |> Maybe.withDefault Dict.empty


getState : StateRef -> String -> Wb (Maybe D.Value)
getState ref key =
    Io.get (stateDict ref >> Dict.get key)


setState : StateRef -> String -> D.Value -> Wb ()
setState ref key value =
    Io.modify
        (\world ->
            ( { world
                | handlers =
                    Dict.update ref.handler
                        (Maybe.map (\state -> { state | states = Dict.update ref.plugin (Maybe.map (Dict.insert key value)) state.states }))
                        world.handlers
              }
            , ()
            )
        )


{-| The next plugin from `from` on, in the strategy's live list, that defines the callback: its
index, the callback, and its state, which a plugin added after the handler was created lacks.
This is a step of Workbox's `iterateCallbacks` generator, which reads `this._strategy.plugins`
at every step.
-}
nextCallback : (Callbacks -> Maybe callback) -> StrategyHandler -> Int -> Wb (Maybe ( Int, callback, Maybe StateRef ))
nextCallback select handler from =
    Io.get
        (\world ->
            let
                hasState pluginId =
                    Dict.get handler.id world.handlers |> Maybe.map (.states >> Dict.member pluginId) |> Maybe.withDefault False

                search index plugins =
                    case plugins of
                        [] ->
                            Nothing

                        (Plugin callbacks) :: rest ->
                            case select callbacks of
                                Just callback ->
                                    Just ( index, callback, if hasState callbacks.id then Just { handler = handler.id, plugin = callbacks.id } else Nothing )

                                Nothing ->
                                    search (index + 1) rest
            in
            search from (List.drop from (pluginsOf handler.strategy world))
        )


{-| `for (const callback of handler.iterateCallbacks(name)) value = await step(callback, value)`,
which stops once `stop` holds for a result.
-}
foldCallbacks : (Callbacks -> Maybe callback) -> StrategyHandler -> (a -> Bool) -> (callback -> Maybe StateRef -> a -> Wb a) -> a -> Wb a
foldCallbacks select handler stop step initial =
    let
        loop from value =
            nextCallback select handler from
                |> Io.andThen
                    (\found ->
                        case found of
                            Nothing ->
                                Io.succeed value

                            Just ( index, callback, state ) ->
                                step callback state value
                                    |> Io.andThen
                                        (\next ->
                                            if stop next then
                                                Io.succeed next

                                            else
                                                loop (index + 1) next
                                        )
                    )
    in
    loop 0 initial


{-| `runCallbacks(name, param)`: each callback in turn.
-}
runCallbacks : (Callbacks -> Maybe callback) -> StrategyHandler -> (callback -> Maybe StateRef -> Wb ()) -> Wb ()
runCallbacks select handler run =
    foldCallbacks select handler (\() -> False) (\callback state () -> run callback state) ()



-- Strategy


{-| `handle(options)`: the response promise of `handleAll`; the completion keeps running.
-}
handle : Strategy -> HandlerCallbackOptions -> Wb (Promise Response)
handle strategy options =
    handleAll strategy options |> Io.map Tuple.first


{-| `handleAll(options)`: a handler for the request, and its response and completion promises,
both already running.
-}
handleAll : Strategy -> HandlerCallbackOptions -> Wb ( Promise Response, Promise () )
handleAll strategy options =
    strategyHandler strategy options
        |> Io.andThen
            (\handler ->
                Io.spawn Host.encodeResponse Host.responseDecoder (getResponse handler)
                    |> Io.andThen
                        (\responseDone ->
                            Io.spawn encodeUnit unitDecoder (awaitComplete responseDone handler)
                                |> Io.map (\handlerDone -> ( responseDone, handlerDone ))
                        )
            )


{-| `_getResponse`: handlerWillStart, `_handle`, handlerDidError on failure, handlerWillRespond.
-}
getResponse : StrategyHandler -> Wb Response
getResponse handler =
    let
        (Strategy strategy) =
            handler.strategy

        request =
            handler.request

        event =
            handler.event

        -- The `catch`: handlerDidError only for an Error, each result replacing the response until
        -- one is a response; then a rethrow when there is none. A response of type "error" stays
        -- the response unless a callback replaces it, so it is returned when none does.
        recover response thrown =
            (if Io.isError thrown then
                foldCallbacks .handlerDidError handler isJust (\callback state _ -> callback { request = request, error = thrown, event = event, state = state }) response

             else
                Io.succeed response
            )
                |> Io.andThen
                    (\recovered ->
                        case recovered of
                            Just found ->
                                Io.succeed found

                            Nothing ->
                                Io.throw thrown
                    )
    in
    runCallbacks .handlerWillStart handler (\callback state -> callback { request = request, event = event, state = state })
        |> Io.andThen (\() -> Io.attempt (strategy.handleRequest request handler))
        |> Io.andThen
            (\outcome ->
                case outcome of
                    Ok response ->
                        if response.kind == "error" then
                            recover (Just response) (workboxError "no-response" [ ( "url", E.string request.url ) ])

                        else
                            Io.succeed response

                    Err thrown ->
                        recover Nothing thrown
            )
        |> Io.andThen (foldCallbacks .handlerWillRespond handler (\_ -> False) (\callback state response -> callback { request = request, response = response, event = event, state = state }))


{-| `_awaitComplete`: handlerDidRespond and the handler's extended lifetimes, then
handlerDidComplete with the Error among them, then the handler's own promise resolves.
-}
awaitComplete : Promise Response -> StrategyHandler -> Wb ()
awaitComplete responseDone handler =
    let
        ( request, event ) =
            ( handler.request, handler.event )
    in
    Io.attempt (Io.await responseDone)
        |> Io.andThen
            (\outcome ->
                let
                    response =
                        Result.toMaybe outcome
                in
                runCallbacks .handlerDidRespond handler (\callback state -> callback { request = request, response = response, event = event, state = state })
                    |> Io.andThen (\() -> doneWaiting handler)
                    |> Io.attempt
                    |> Io.andThen
                        (\waited ->
                            let
                                error =
                                    case waited of
                                        Err thrown ->
                                            if Io.isError thrown then
                                                Just thrown

                                            else
                                                Nothing

                                        Ok () ->
                                            Nothing
                            in
                            runCallbacks .handlerDidComplete handler (\callback state -> callback { request = request, response = response, error = error, event = event, state = state })
                                |> Io.andThen (\() -> destroy handler)
                                |> Io.andThen (\() -> error |> Maybe.map Io.throw |> Maybe.withDefault (Io.succeed ()))
                        )
            )



-- StrategyHandler


{-| `new StrategyHandler(strategy, options)`: each plugin present now gets a fresh state, and the
handler's own promise extends the event's lifetime.
-}
strategyHandler : Strategy -> HandlerCallbackOptions -> Wb StrategyHandler
strategyHandler strategy options =
    World.newId
        |> Io.andThen
            (\id ->
                Io.deferred unitDecoder
                    |> Io.andThen
                        (\handlerDeferred ->
                            Io.modify
                                (\world ->
                                    let
                                        states =
                                            pluginsOf strategy world |> List.map (\plugin -> ( World.pluginId plugin, Dict.empty )) |> Dict.fromList

                                        state =
                                            { cacheKeys = Dict.empty, extendLifetime = [], states = states, deferred = Io.promiseId handlerDeferred }
                                    in
                                    ( { world | handlers = Dict.insert id state world.handlers }, () )
                                )
                                |> Io.andThen (\() -> World.eventWaitUntil options.event (Io.promiseId handlerDeferred))
                                |> Io.map (\() -> { id = id, strategy = strategy, request = options.request, event = options.event, params = options.params })
                        )
            )


{-| `fetch(input)`: requestWillFetch, the network, then fetchDidSucceed or fetchDidFail.
-}
fetch : StrategyHandler -> Request -> Wb Response
fetch handler input =
    let
        event =
            handler.event

        -- The `try` around the requestWillFetch loop: an Error becomes
        -- plugin-error-request-will-fetch, and a thrown non-Error ends the loop with the request
        -- as it is.
        requestWillFetch from request =
            nextCallback .requestWillFetch handler from
                |> Io.andThen
                    (\found ->
                        case found of
                            Nothing ->
                                Io.succeed request

                            Just ( index, callback, state ) ->
                                Host.cloneRequest request
                                    |> Io.andThen (\copy -> callback { request = copy, event = event, state = state })
                                    |> Io.attempt
                                    |> Io.andThen
                                        (\outcome ->
                                            case outcome of
                                                Ok next ->
                                                    requestWillFetch (index + 1) next

                                                Err thrown ->
                                                    if Io.isError thrown then
                                                        Io.throw (workboxError "plugin-error-request-will-fetch" [ ( "thrownErrorMessage", E.string (messageOf thrown) ) ])

                                                    else
                                                        Io.succeed request
                                        )
                    )
    in
    hasCallback .fetchDidFail handler.strategy
        |> Io.andThen (\hasFetchDidFail -> if hasFetchDidFail then Host.cloneRequest input |> Io.map Just else Io.succeed Nothing)
        |> Io.andThen
            (\originalRequest ->
                requestWillFetch 0 input
                    |> Io.andThen
                        (\request ->
                            Host.cloneRequest request
                                |> Io.andThen
                                    (\pluginFilteredRequest ->
                                        Host.fetch request
                                            |> Io.andThen
                                                (foldCallbacks .fetchDidSucceed handler (\_ -> False) (\callback state response -> callback { request = pluginFilteredRequest, response = response, event = event, state = state }))
                                            |> Io.catch
                                                (\thrown ->
                                                    (case originalRequest of
                                                        Just original ->
                                                            -- One clone of each for every callback, as runCallbacks passes one param object.
                                                            Host.cloneRequest original
                                                                |> Io.andThen
                                                                    (\originalClone ->
                                                                        Host.cloneRequest pluginFilteredRequest
                                                                            |> Io.andThen
                                                                                (\filteredClone ->
                                                                                    runCallbacks .fetchDidFail handler (\callback state -> callback { error = thrown, originalRequest = originalClone, request = filteredClone, event = event, state = state })
                                                                                )
                                                                    )

                                                        Nothing ->
                                                            Io.succeed ()
                                                    )
                                                        |> Io.andThen (\() -> Io.throw thrown)
                                                )
                                    )
                        )
            )


{-| An Error's `message`.
-}
messageOf : Thrown -> String
messageOf thrown =
    case thrown of
        WorkboxError name details ->
            Io.workboxMessage name details

        HostError error ->
            error.message


{-| `fetchAndCachePut(input)`: the response now, and its cachePut as an extended lifetime.
-}
fetchAndCachePut : StrategyHandler -> Request -> Wb Response
fetchAndCachePut handler input =
    fetch handler input
        |> Io.andThen
            (\response ->
                Host.cloneResponse response
                    |> Io.andThen (\responseClone -> Io.spawn E.bool D.bool (cachePut handler input responseClone))
                    |> Io.andThen (\cached -> handlerWaitUntil handler (Io.promiseId cached))
                    |> Io.map (\() -> response)
            )


{-| `cacheMatch(key)`: `caches.match` of the read cache key in the strategy's cache (which opens
no cache), through cachedResponseWillBeUsed.
-}
cacheMatch : StrategyHandler -> Request -> Wb (Maybe Response)
cacheMatch handler key =
    let
        cacheName =
            World.strategyCacheName handler.strategy
    in
    getCacheKey handler key "read"
        |> Io.andThen
            (\effectiveRequest ->
                Host.cachesMatch effectiveRequest cacheName
                    |> Io.andThen
                        (foldCallbacks .cachedResponseWillBeUsed
                            handler
                            (\_ -> False)
                            (\callback state cachedResponse -> callback { cacheName = cacheName, request = effectiveRequest, cachedResponse = cachedResponse, event = handler.event, state = state })
                        )
            )


{-| `cachePut(key, response)`: one timer task later, the write cache key, cacheWillUpdate (or the
status-200 rule), the put, then cacheDidUpdate. Answers whether it cached.
-}
cachePut : StrategyHandler -> Request -> Response -> Wb Bool
cachePut handler key response =
    WorkboxCore.timeout
        |> Io.andThen (\() -> getCacheKey handler key "write")
        |> Io.andThen
            (\effectiveRequest ->
                ensureResponseSafeToCache handler response
                    |> Io.andThen
                        (\safe ->
                            case safe of
                                Nothing ->
                                    Io.succeed False

                                Just responseToCache ->
                                    put handler effectiveRequest responseToCache |> Io.map (\() -> True)
                        )
            )


put : StrategyHandler -> Request -> Response -> Wb ()
put handler effectiveRequest responseToCache =
    let
        cacheName =
            World.strategyCacheName handler.strategy
    in
    Host.cachesOpen cacheName
        |> Io.andThen
            (\cache ->
                hasCallback .cacheDidUpdate handler.strategy
                    |> Io.andThen
                        (\hasCacheUpdateCallback ->
                            (if hasCacheUpdateCallback then
                                Host.cloneRequest effectiveRequest |> Io.andThen (\copy -> cacheMatchIgnoreParams cache copy [ "__WB_REVISION__" ])

                             else
                                Io.succeed Nothing
                            )
                                |> Io.andThen
                                    (\oldResponse ->
                                        (if hasCacheUpdateCallback then
                                            Host.cloneResponse responseToCache

                                         else
                                            Io.succeed responseToCache
                                        )
                                            |> Io.andThen (Host.cachePut cache effectiveRequest)
                                            -- An Error is rethrown (a QuotaExceededError after the registered quota
                                            -- callbacks, of which the subset has none); anything else is ignored.
                                            |> Io.catch (\thrown -> if Io.isError thrown then Io.throw thrown else Io.succeed ())
                                            |> Io.andThen
                                                (\() ->
                                                    runCallbacks .cacheDidUpdate
                                                        handler
                                                        (\callback state ->
                                                            Host.cloneResponse responseToCache
                                                                |> Io.andThen (\newResponse -> callback { cacheName = cacheName, oldResponse = oldResponse, newResponse = newResponse, request = effectiveRequest, event = handler.event, state = state })
                                                        )
                                                )
                                    )
                        )
            )


{-| `getCacheKey(request, mode)`: cacheKeyWillBeUsed once per URL and mode for this handler.
-}
getCacheKey : StrategyHandler -> Request -> String -> Wb Request
getCacheKey handler request mode =
    let
        key =
            request.url ++ " | " ++ mode
    in
    Io.get (\world -> Dict.get handler.id world.handlers |> Maybe.andThen (.cacheKeys >> Dict.get key))
        |> Io.andThen
            (\known ->
                case known of
                    Just effectiveRequest ->
                        Io.succeed effectiveRequest

                    Nothing ->
                        foldCallbacks .cacheKeyWillBeUsed
                            handler
                            (\_ -> False)
                            (\callback state effectiveRequest -> callback { mode = mode, request = effectiveRequest, event = handler.event, params = handler.params, state = state })
                            request
                            |> Io.andThen
                                (\effectiveRequest ->
                                    Io.modify
                                        (\world ->
                                            ( { world | handlers = Dict.update handler.id (Maybe.map (\state -> { state | cacheKeys = Dict.insert key effectiveRequest state.cacheKeys })) world.handlers }
                                            , effectiveRequest
                                            )
                                        )
                                )
            )


{-| `_ensureResponseSafeToCache(response)`: every cacheWillUpdate in turn until one refuses, or,
without any, only a status 200.
-}
ensureResponseSafeToCache : StrategyHandler -> Response -> Wb (Maybe Response)
ensureResponseSafeToCache handler response =
    let
        loop from responseToCache pluginsUsed =
            nextCallback .cacheWillUpdate handler from
                |> Io.andThen
                    (\found ->
                        case ( found, responseToCache ) of
                            ( Just ( index, callback, state ), Just current ) ->
                                callback { request = handler.request, response = current, event = handler.event, state = state }
                                    |> Io.andThen (\next -> loop (index + 1) next True)

                            _ ->
                                Io.succeed
                                    (if pluginsUsed then
                                        responseToCache

                                     else
                                        responseToCache |> Maybe.andThen (\current -> if current.status == 200 then Just current else Nothing)
                                    )
                    )
    in
    loop 0 (Just response) False


{-| `waitUntil(promise)`: extends the handler's lifetime until the promise settles.
-}
handlerWaitUntil : StrategyHandler -> Int -> Wb ()
handlerWaitUntil handler promise =
    Io.modify (\world -> ( { world | handlers = Dict.update handler.id (Maybe.map (\state -> { state | extendLifetime = state.extendLifetime ++ [ promise ] })) world.handlers }, () ))


{-| `doneWaiting()`: settles every extended lifetime, including ones added meanwhile, and throws
the first rejection of a batch.
-}
doneWaiting : StrategyHandler -> Wb ()
doneWaiting handler =
    Io.modify
        (\world ->
            case Dict.get handler.id world.handlers of
                Just state ->
                    ( { world | handlers = Dict.insert handler.id { state | extendLifetime = [] } world.handlers }, state.extendLifetime )

                Nothing ->
                    ( world, [] )
        )
        |> Io.andThen
            (\batch ->
                if List.isEmpty batch then
                    Io.succeed ()

                else
                    Io.sequence (List.map (\promise -> Io.Await promise Io.Done) batch)
                        |> Io.andThen
                            (\outcomes ->
                                case List.filterMap (\outcome -> Result.toMaybe (swap outcome)) outcomes of
                                    thrown :: _ ->
                                        Io.throw thrown

                                    [] ->
                                        doneWaiting handler
                            )
            )


swap : Result e a -> Result a e
swap result =
    case result of
        Ok value ->
            Err value

        Err error ->
            Ok error


destroy : StrategyHandler -> Wb ()
destroy handler =
    Io.get (\world -> Dict.get handler.id world.handlers |> Maybe.map .deferred)
        |> Io.andThen
            (\found ->
                case found of
                    Just promise ->
                        Io.Settle promise (Ok E.null) (Io.Done ())

                    Nothing ->
                        Io.succeed ()
            )



-- The strategies


{-| `new WorkboxError('no-response', {url, error})`; the caught value is kept only when it is an
Error (`error` is then left out of the details).
-}
noResponse : Request -> Maybe Thrown -> Thrown
noResponse request error =
    workboxError "no-response" (( "url", E.string request.url ) :: (error |> Maybe.map (\thrown -> [ ( "error", Io.asDetail thrown ) ]) |> Maybe.withDefault []))


{-| The caught value of a strategy's `catch`, kept only when it is an Error.
-}
keptError : Thrown -> Maybe Thrown
keptError thrown =
    if Io.isError thrown then
        Just thrown

    else
        Nothing


cacheFirst : String -> List Plugin -> Wb Strategy
cacheFirst cacheName plugins =
    newStrategy cacheName
        plugins
        (\request handler ->
            cacheMatch handler request
                |> Io.andThen
                    (\cached ->
                        case cached of
                            Just response ->
                                Io.succeed response

                            Nothing ->
                                Io.attempt (fetchAndCachePut handler request)
                                    |> Io.andThen
                                        (\outcome ->
                                            case outcome of
                                                Ok response ->
                                                    Io.succeed response

                                                Err thrown ->
                                                    Io.throw (noResponse request (keptError thrown))
                                        )
                    )
        )


cacheOnly : String -> List Plugin -> Wb Strategy
cacheOnly cacheName plugins =
    newStrategy cacheName
        plugins
        (\request handler ->
            cacheMatch handler request
                |> Io.andThen (Maybe.map Io.succeed >> Maybe.withDefault (Io.throw (noResponse request Nothing)))
        )


{-| `cacheOkAndOpaquePlugin`: status 200, or 0 for an opaque response.
-}
cacheOkAndOpaquePlugin : Plugin
cacheOkAndOpaquePlugin =
    let
        callbacks =
            World.noCallbacks 1
    in
    Plugin { callbacks | cacheWillUpdate = Just (\param -> Io.succeed (if param.response.status == 200 || param.response.status == 0 then Just param.response else Nothing)) }


{-| `NetworkFirst` without `networkTimeoutSeconds`. Its constructor puts the 200-or-opaque plugin
first unless a plugin has a cacheWillUpdate.
-}
networkFirst : String -> List Plugin -> Wb Strategy
networkFirst cacheName plugins =
    let
        withCacheability =
            if List.any (\(Plugin callbacks) -> isJust callbacks.cacheWillUpdate) plugins then
                plugins

            else
                cacheOkAndOpaquePlugin :: plugins

        maybeResponseDecoder =
            D.nullable Host.responseDecoder
    in
    newStrategy cacheName
        withCacheability
        (\request handler ->
            Io.spawn encodeMaybeResponse maybeResponseDecoder (getNetworkPromise request handler)
                |> Io.andThen
                    (\networkPromise ->
                        -- `(async () => await handler.waitUntil(Promise.race([a])) || await a)()`, whose
                        -- promise also extends the handler's lifetime; a race of one settles as `a`.
                        Io.spawn encodeMaybeResponse
                            maybeResponseDecoder
                            (Io.spawn encodeMaybeResponse maybeResponseDecoder (Io.await networkPromise)
                                |> Io.andThen (\race -> handlerWaitUntil handler (Io.promiseId race) |> Io.andThen (\() -> Io.await race))
                                |> Io.andThen (\raced -> if isJust raced then Io.succeed raced else Io.await networkPromise)
                            )
                            |> Io.andThen (\responsePromise -> handlerWaitUntil handler (Io.promiseId responsePromise) |> Io.andThen (\() -> Io.await responsePromise))
                    )
                |> Io.andThen (Maybe.map Io.succeed >> Maybe.withDefault (Io.throw (noResponse request Nothing)))
        )


{-| `_getNetworkPromise`: fetchAndCachePut, and the cache when that failed or answered nothing.
-}
getNetworkPromise : Request -> StrategyHandler -> Wb (Maybe Response)
getNetworkPromise request handler =
    Io.attempt (fetchAndCachePut handler request)
        |> Io.andThen
            (\outcome ->
                case outcome of
                    Ok response ->
                        Io.succeed (Just response)

                    Err _ ->
                        cacheMatch handler request
            )


networkOnly : String -> List Plugin -> Wb Strategy
networkOnly cacheName plugins =
    newStrategy cacheName
        plugins
        (\request handler ->
            Io.attempt (fetch handler request)
                |> Io.andThen
                    (\outcome ->
                        case outcome of
                            Ok response ->
                                Io.succeed response

                            Err thrown ->
                                Io.throw (noResponse request (keptError thrown))
                    )
        )
