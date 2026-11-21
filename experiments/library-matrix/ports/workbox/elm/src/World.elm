module World exposing
    ( World, Wb, Location, initial, newId
    , Plugin(..), Callbacks, noCallbacks, pluginId
    , StateRef, HandlerWillStartParam, CacheKeyWillBeUsedParam, CachedResponseWillBeUsedParam, CacheWillUpdateParam
    , CacheDidUpdateParam, RequestWillFetchParam, FetchDidSucceedParam, FetchDidFailParam, HandlerWillRespondParam
    , HandlerDidRespondParam, HandlerDidCompleteParam, HandlerDidErrorParam
    , Event(..), eventKind, eventWaitUntil
    , Params, noParams, HandlerCallbackOptions, RouteHandler(..), handleWith
    , Strategy(..), strategyId, strategyCacheName, StrategyHandler, HandlerState
    , PrecacheController, ControllerState, InstallReport
    , Route(..), MatchCallbackOptions, Router, RouterState
    , Fixture, Trace
    )

{-| The program's heap: what JavaScript Workbox keeps in mutable objects (a strategy's plugin list,
a handler's cache keys, lifetimes and plugin states, a controller's maps, a router's routes and
handlers, an install report, a stand-in event's lifetimes), and the adapter's fixture state that
the fixture plugins share. Objects are ids into it; a plugin's identity is its id.

The types Workbox's layers pass between them live here too, as Elm cannot have modules import
each other.

-}

import Dict exposing (Dict)
import Host exposing (Request, Response, Url)
import Io exposing (Io, Promise, Thrown)
import Json.Decode as D


type alias Wb a =
    Io World a


type alias Location =
    { href : String
    , origin : String
    }


type alias World =
    { nextId : Int
    , location : Location
    , strategies : Dict Int (List Plugin)
    , handlers : Dict Int HandlerState
    , controllers : Dict Int ControllerState
    , reports : Dict Int InstallReport
    , routers : Dict Int RouterState
    , routeCatchHandlers : Dict Int RouteHandler
    , standInEvents : Dict Int (List Int)
    , fixture : Fixture
    }


{-| The adapter's state that its steps and fixture plugins share: the location base and the cache
prefix, the network fixture's source, the handler traces (newest first), each trace's records
(newest first), and the `[type, count]` of each event-driven step (newest first).
-}
type alias Fixture =
    { base : String
    , prefix : String
    , source : String
    , traces : List Trace
    , logs : Dict Int (List D.Value)
    , events : List D.Value
    }


type alias Trace =
    { log : Int
    , label : String
    , url : String
    }


{-| The ids below 100 name Workbox's module-level plugins.
-}
initial : Location -> Fixture -> World
initial location fixture =
    { nextId = 100
    , location = location
    , strategies = Dict.empty
    , handlers = Dict.empty
    , controllers = Dict.empty
    , reports = Dict.empty
    , routers = Dict.empty
    , routeCatchHandlers = Dict.empty
    , standInEvents = Dict.empty
    , fixture = fixture
    }


newId : Wb Int
newId =
    Io.modify (\world -> ( { world | nextId = world.nextId + 1 }, world.nextId ))



-- Plugins


{-| `WorkboxPlugin`: an object of optional lifecycle callbacks; Workbox calls only those a plugin
defines.
-}
type Plugin
    = Plugin Callbacks


type alias Callbacks =
    { id : Int
    , handlerWillStart : Maybe (HandlerWillStartParam -> Wb ())
    , cacheKeyWillBeUsed : Maybe (CacheKeyWillBeUsedParam -> Wb Request)
    , cachedResponseWillBeUsed : Maybe (CachedResponseWillBeUsedParam -> Wb (Maybe Response))
    , cacheWillUpdate : Maybe (CacheWillUpdateParam -> Wb (Maybe Response))
    , cacheDidUpdate : Maybe (CacheDidUpdateParam -> Wb ())
    , requestWillFetch : Maybe (RequestWillFetchParam -> Wb Request)
    , fetchDidSucceed : Maybe (FetchDidSucceedParam -> Wb Response)
    , fetchDidFail : Maybe (FetchDidFailParam -> Wb ())
    , handlerWillRespond : Maybe (HandlerWillRespondParam -> Wb Response)
    , handlerDidRespond : Maybe (HandlerDidRespondParam -> Wb ())
    , handlerDidComplete : Maybe (HandlerDidCompleteParam -> Wb ())
    , handlerDidError : Maybe (HandlerDidErrorParam -> Wb (Maybe Response))
    }


noCallbacks : Int -> Callbacks
noCallbacks id =
    { id = id
    , handlerWillStart = Nothing
    , cacheKeyWillBeUsed = Nothing
    , cachedResponseWillBeUsed = Nothing
    , cacheWillUpdate = Nothing
    , cacheDidUpdate = Nothing
    , requestWillFetch = Nothing
    , fetchDidSucceed = Nothing
    , fetchDidFail = Nothing
    , handlerWillRespond = Nothing
    , handlerDidRespond = Nothing
    , handlerDidComplete = Nothing
    , handlerDidError = Nothing
    }


pluginId : Plugin -> Int
pluginId (Plugin callbacks) =
    callbacks.id


{-| The state object a handler keeps for one plugin: which handler, which plugin.
-}
type alias StateRef =
    { handler : Int
    , plugin : Int
    }


type alias HandlerWillStartParam =
    { request : Request, event : Event, state : Maybe StateRef }


type alias CacheKeyWillBeUsedParam =
    { mode : String, request : Request, event : Event, params : Maybe Params, state : Maybe StateRef }


type alias CachedResponseWillBeUsedParam =
    { cacheName : String, request : Request, cachedResponse : Maybe Response, event : Event, state : Maybe StateRef }


type alias CacheWillUpdateParam =
    { request : Request, response : Response, event : Event, state : Maybe StateRef }


type alias CacheDidUpdateParam =
    { cacheName : String, oldResponse : Maybe Response, newResponse : Response, request : Request, event : Event, state : Maybe StateRef }


type alias RequestWillFetchParam =
    { request : Request, event : Event, state : Maybe StateRef }


type alias FetchDidSucceedParam =
    { request : Request, response : Response, event : Event, state : Maybe StateRef }


type alias FetchDidFailParam =
    { error : Thrown, originalRequest : Request, request : Request, event : Event, state : Maybe StateRef }


type alias HandlerWillRespondParam =
    { request : Request, response : Response, event : Event, state : Maybe StateRef }


type alias HandlerDidRespondParam =
    { request : Request, response : Maybe Response, event : Event, state : Maybe StateRef }


type alias HandlerDidCompleteParam =
    { request : Request, response : Maybe Response, error : Maybe Thrown, event : Event, state : Maybe StateRef }


type alias HandlerDidErrorParam =
    { request : Request, error : Thrown, event : Event, state : Maybe StateRef }



-- Events


{-| The event a strategy, `install` or `activate` runs under: its `type`, and `waitUntil`, which
takes a promise by id.
-}
type Event
    = Event { kind : String, waitUntil : Int -> Wb () }


eventKind : Event -> String
eventKind (Event event) =
    event.kind


eventWaitUntil : Event -> Int -> Wb ()
eventWaitUntil (Event event) =
    event.waitUntil



-- Routing and strategies


{-| The route parameters a handler receives: `{cacheKey, integrity}` from a PrecacheRoute match,
`{cacheKey}` from createHandlerBoundToURL.
-}
type alias Params =
    { cacheKey : Maybe String
    , integrity : Maybe String
    }


noParams : Params
noParams =
    { cacheKey = Nothing, integrity = Nothing }


type alias HandlerCallbackOptions =
    { request : Request
    , event : Event
    , params : Maybe Params
    }


{-| A route handler as `normalizeHandler` leaves it: `handle(options)`, which answers a promise.
-}
type RouteHandler
    = RouteHandler (HandlerCallbackOptions -> Wb (Promise Response))


handleWith : RouteHandler -> HandlerCallbackOptions -> Wb (Promise Response)
handleWith (RouteHandler handle) =
    handle


{-| `Strategy`: its cache name and the subclass's `_handle`. Its plugin list is in the world.
-}
type Strategy
    = Strategy
        { id : Int
        , cacheName : String
        , handleRequest : Request -> StrategyHandler -> Wb Response
        }


strategyId : Strategy -> Int
strategyId (Strategy strategy) =
    strategy.id


strategyCacheName : Strategy -> String
strategyCacheName (Strategy strategy) =
    strategy.cacheName


{-| `StrategyHandler`, the part that does not change; the rest is its `HandlerState`.
-}
type alias StrategyHandler =
    { id : Int
    , strategy : Strategy
    , request : Request
    , event : Event
    , params : Maybe Params
    }


{-| A handler's cache keys per `url | mode`, its extended lifetimes (oldest first), each plugin's
state (by plugin id: the plugins present when it was created), and its own promise.
-}
type alias HandlerState =
    { cacheKeys : Dict String Request
    , extendLifetime : List Int
    , states : Dict Int (Dict String D.Value)
    , deferred : Int
    }


type alias PrecacheController =
    { id : Int
    , strategy : Strategy
    }


{-| The controller's three maps. `urls` is the insertion order of `urlsToCacheKeys`, which
re-setting a URL does not change.
-}
type alias ControllerState =
    { urls : List String
    , urlsToCacheKeys : Dict String String
    , urlsToCacheModes : Dict String String
    , cacheKeysToIntegrities : Dict String String
    }


{-| An install report, newest URL first.
-}
type alias InstallReport =
    { updatedURLs : List String
    , notUpdatedURLs : List String
    }


type alias MatchCallbackOptions =
    { url : Url
    , sameOrigin : Bool
    , request : Request
    , event : Event
    }


{-| `new Route(match, handler, method)`. A match answers Nothing for no match, `Just Nothing` for
a match without params (`true`, `{}`), or the params. Its catch handler is in the world.
-}
type Route
    = Route
        { id : Int
        , match : MatchCallbackOptions -> Wb (Maybe (Maybe Params))
        , handler : RouteHandler
        , method : String
        }


type alias Router =
    { id : Int
    }


type alias RouterState =
    { routes : Dict String (List Route)
    , defaultHandlerMap : Dict String RouteHandler
    , catchHandler : Maybe RouteHandler
    }
