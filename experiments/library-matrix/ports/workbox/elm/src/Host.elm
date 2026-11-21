module Host exposing
    ( Request, Response, Url, Cache, Headers, Blob, RegExp
    , RequestInit, requestInit
    , locationHref, locationOrigin, randomUUID, warn
    , newUrl, setPathname, setHash, searchParamKeys, searchParamSet, searchParamDelete
    , newRegExp, regExpTest
    , newRequest, requestFromRequest, cloneRequest, requestHeaders
    , newResponse, responseFromParts, cloneResponse, responseHeaders, responseText, responseBlob, markRedirected
    , cachesOpen, cachesMatch, cachesKeys, cachesDelete
    , cacheMatch, cacheMatchUrl, cachePut, cacheDelete, cacheKeys, cacheKeysIgnoringSearch
    , fetch
    , eventType, eventWaitUntil, settleLifetime
    , encodeRequest, requestDecoder, encodeResponse, responseDecoder
    )

{-| The contract's host primitives (CONTRACT.md "Elm: the I/O boundary"), as `Io` actions. The JS
host (`js/index.js`) performs each one and answers with what it gave or threw; host objects stay
in the host, behind handles. A Request, Response or URL comes back with the properties Workbox
reads, so a read is not a round trip.
-}

import Io exposing (Io)
import Json.Decode as D
import Json.Encode as E


type alias Request =
    { handle : Int
    , url : String
    , method : String
    , mode : String
    , integrity : String
    }


type alias Response =
    { handle : Int
    , status : Int
    , statusText : String
    , url : String
    , redirected : Bool
    , kind : String
    }


{-| A WHATWG URL as it is now; a write answers the URL as it is after the write.
-}
type alias Url =
    { handle : Int
    , href : String
    , origin : String
    , protocol : String
    , pathname : String
    }


type Cache
    = Cache Int


type Headers
    = Headers Int


type Blob
    = Blob Int


type RegExp
    = RegExp Int


handle : Int -> E.Value
handle =
    E.int


requestDecoder : D.Decoder Request
requestDecoder =
    D.map5 Request
        (D.field "handle" D.int)
        (D.field "url" D.string)
        (D.field "method" D.string)
        (D.field "mode" D.string)
        (D.field "integrity" D.string)


encodeRequest : Request -> E.Value
encodeRequest request =
    E.object
        [ ( "handle", E.int request.handle )
        , ( "url", E.string request.url )
        , ( "method", E.string request.method )
        , ( "mode", E.string request.mode )
        , ( "integrity", E.string request.integrity )
        ]


responseDecoder : D.Decoder Response
responseDecoder =
    D.map6 Response
        (D.field "handle" D.int)
        (D.field "status" D.int)
        (D.field "statusText" D.string)
        (D.field "url" D.string)
        (D.field "redirected" D.bool)
        (D.field "type" D.string)


encodeResponse : Response -> E.Value
encodeResponse response =
    E.object
        [ ( "handle", E.int response.handle )
        , ( "status", E.int response.status )
        , ( "statusText", E.string response.statusText )
        , ( "url", E.string response.url )
        , ( "redirected", E.bool response.redirected )
        , ( "type", E.string response.kind )
        ]


urlDecoder : D.Decoder Url
urlDecoder =
    D.map5 Url
        (D.field "handle" D.int)
        (D.field "href" D.string)
        (D.field "origin" D.string)
        (D.field "protocol" D.string)
        (D.field "pathname" D.string)



-- Environment and text


locationHref : Io w String
locationHref =
    Io.call "locationHref" [] D.string


locationOrigin : Io w String
locationOrigin =
    Io.call "locationOrigin" [] D.string


randomUUID : Io w String
randomUUID =
    Io.call "randomUUID" [] D.string


warn : String -> Io w ()
warn message =
    Io.call "warn" [ ( "message", E.string message ) ] (D.succeed ())



-- URL


{-| `new URL(input, base)`, or the TypeError it throws.
-}
newUrl : String -> Maybe String -> Io w Url
newUrl input base =
    Io.call "newUrl" [ ( "input", E.string input ), ( "base", base |> Maybe.map E.string |> Maybe.withDefault E.null ) ] urlDecoder


setPathname : Url -> String -> Io w Url
setPathname url value =
    Io.call "setPathname" [ ( "url", handle url.handle ), ( "value", E.string value ) ] urlDecoder


setHash : Url -> String -> Io w Url
setHash url value =
    Io.call "setHash" [ ( "url", handle url.handle ), ( "value", E.string value ) ] urlDecoder


{-| `[...url.searchParams.keys()]`.
-}
searchParamKeys : Url -> Io w (List String)
searchParamKeys url =
    Io.call "searchParamKeys" [ ( "url", handle url.handle ) ] (D.list D.string)


{-| `url.searchParams.set(name, value)`: the host converts a number with ToString.
-}
searchParamSet : Url -> String -> E.Value -> Io w Url
searchParamSet url name value =
    Io.call "searchParamSet" [ ( "url", handle url.handle ), ( "name", E.string name ), ( "value", value ) ] urlDecoder


searchParamDelete : Url -> String -> Io w Url
searchParamDelete url name =
    Io.call "searchParamDelete" [ ( "url", handle url.handle ), ( "name", E.string name ) ] urlDecoder



-- RegExp


{-| `new RegExp(source)`, JavaScript's semantics without flags, or the SyntaxError it throws.
-}
newRegExp : String -> Io w RegExp
newRegExp source =
    Io.call "newRegExp" [ ( "source", E.string source ) ] (D.map RegExp D.int)


regExpTest : RegExp -> String -> Io w Bool
regExpTest (RegExp regExp) input =
    Io.call "regExpTest" [ ( "regExp", handle regExp ), ( "input", E.string input ) ] D.bool



-- Request


{-| The members of `new Request(input, init)` the subset uses; an absent one stays out of the
dictionary.
-}
type alias RequestInit =
    { method : Maybe String
    , mode : Maybe String
    , integrity : Maybe String
    , cache : Maybe String
    , credentials : Maybe String
    , headers : Maybe Headers
    }


requestInit : RequestInit
requestInit =
    { method = Nothing, mode = Nothing, integrity = Nothing, cache = Nothing, credentials = Nothing, headers = Nothing }


encodeInit : RequestInit -> E.Value
encodeInit init =
    let
        member name =
            Maybe.map (\value -> ( name, E.string value ))
    in
    E.object
        (List.filterMap identity
            [ member "method" init.method
            , member "mode" init.mode
            , member "integrity" init.integrity
            , member "cache" init.cache
            , member "credentials" init.credentials
            , init.headers |> Maybe.map (\(Headers headers) -> ( "headers", handle headers ))
            ]
        )


{-| `new Request(url, init)`, or the TypeError it throws.
-}
newRequest : String -> RequestInit -> Io w Request
newRequest url init =
    Io.call "newRequest" [ ( "input", E.string url ), ( "init", encodeInit init ) ] requestDecoder


{-| `new Request(request, init)`: a copy of the request with `init` applied.
-}
requestFromRequest : Request -> RequestInit -> Io w Request
requestFromRequest request init =
    Io.call "newRequest" [ ( "input", handle request.handle ), ( "init", encodeInit init ) ] requestDecoder


cloneRequest : Request -> Io w Request
cloneRequest request =
    Io.call "cloneRequest" [ ( "request", handle request.handle ) ] requestDecoder


requestHeaders : Request -> Io w Headers
requestHeaders request =
    Io.call "requestHeaders" [ ( "request", handle request.handle ) ] (D.map Headers D.int)



-- Response


{-| `new Response(body, {status})`, or the RangeError it throws.
-}
newResponse : E.Value -> E.Value -> Io w Response
newResponse body status =
    Io.call "newResponse" [ ( "body", body ), ( "status", status ) ] responseDecoder


{-| `new Response(blob, {headers: new Headers(headers), status, statusText})`.
-}
responseFromParts : Blob -> Headers -> Int -> String -> Io w Response
responseFromParts (Blob blob) (Headers headers) status statusText =
    Io.call "responseFromParts"
        [ ( "blob", handle blob ), ( "headers", handle headers ), ( "status", E.int status ), ( "statusText", E.string statusText ) ]
        responseDecoder


cloneResponse : Response -> Io w Response
cloneResponse response =
    Io.call "cloneResponse" [ ( "response", handle response.handle ) ] responseDecoder


responseHeaders : Response -> Io w Headers
responseHeaders response =
    Io.call "responseHeaders" [ ( "response", handle response.handle ) ] (D.map Headers D.int)


responseText : Response -> Io w String
responseText response =
    Io.start "responseText" [ ( "response", handle response.handle ) ] D.string


responseBlob : Response -> Io w Blob
responseBlob response =
    Io.start "responseBlob" [ ( "response", handle response.handle ) ] (D.map Blob D.int)


{-| The network fixture's `redirected` source: own `redirected: true`, and an own `clone` that
marks its result.
-}
markRedirected : Response -> Io w Response
markRedirected response =
    Io.call "markRedirected" [ ( "response", handle response.handle ) ] responseDecoder



-- CacheStorage and Cache


cachesOpen : String -> Io w Cache
cachesOpen name =
    Io.start "cachesOpen" [ ( "name", E.string name ) ] (D.map Cache D.int)


{-| `caches.match(request, {cacheName})`, which opens no cache.
-}
cachesMatch : Request -> String -> Io w (Maybe Response)
cachesMatch request cacheName =
    Io.start "cachesMatch" [ ( "request", handle request.handle ), ( "cacheName", E.string cacheName ) ] (D.nullable responseDecoder)


cachesKeys : Io w (List String)
cachesKeys =
    Io.start "cachesKeys" [] (D.list D.string)


cachesDelete : String -> Io w Bool
cachesDelete name =
    Io.start "cachesDelete" [ ( "name", E.string name ) ] D.bool


cacheMatch : Cache -> Request -> Io w (Maybe Response)
cacheMatch (Cache cache) request =
    Io.start "cacheMatch" [ ( "cache", handle cache ), ( "request", handle request.handle ) ] (D.nullable responseDecoder)


cacheMatchUrl : Cache -> String -> Io w (Maybe Response)
cacheMatchUrl (Cache cache) url =
    Io.start "cacheMatch" [ ( "cache", handle cache ), ( "request", E.string url ) ] (D.nullable responseDecoder)


cachePut : Cache -> Request -> Response -> Io w ()
cachePut (Cache cache) request response =
    Io.start "cachePut" [ ( "cache", handle cache ), ( "request", handle request.handle ), ( "response", handle response.handle ) ] (D.succeed ())


cacheDelete : Cache -> Request -> Io w Bool
cacheDelete (Cache cache) request =
    Io.start "cacheDelete" [ ( "cache", handle cache ), ( "request", handle request.handle ) ] D.bool


cacheKeys : Cache -> Io w (List Request)
cacheKeys (Cache cache) =
    Io.start "cacheKeys" [ ( "cache", handle cache ), ( "request", E.null ) ] (D.list requestDecoder)


{-| `cache.keys(request, {ignoreSearch: true})`.
-}
cacheKeysIgnoringSearch : Cache -> Request -> Io w (List Request)
cacheKeysIgnoringSearch (Cache cache) request =
    Io.start "cacheKeys" [ ( "cache", handle cache ), ( "request", handle request.handle ) ] (D.list requestDecoder)



-- Fetch


fetch : Request -> Io w Response
fetch request =
    Io.start "fetch" [ ( "request", handle request.handle ) ] responseDecoder



-- A native ExtendableEvent (service-worker mode)


eventType : Io w String
eventType =
    Io.call "eventType" [] D.string


{-| `event.waitUntil(promise)` with a promise the host settles when told to: the lifetime token.
-}
eventWaitUntil : Io w Int
eventWaitUntil =
    Io.call "eventWaitUntil" [] D.int


{-| Settles a lifetime: fulfilled, or rejected with the failure's name and message.
-}
settleLifetime : Int -> Maybe ( String, String ) -> Io w ()
settleLifetime token failure =
    Io.call "settleLifetime"
        [ ( "token", E.int token )
        , ( "failure", failure |> Maybe.map (\( name, message ) -> E.object [ ( "name", E.string name ), ( "message", E.string message ) ]) |> Maybe.withDefault E.null )
        ]
        (D.succeed ())
