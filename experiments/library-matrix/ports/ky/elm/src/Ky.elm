module Ky exposing (Failure(..), Hooks, Options, RetryOptions, Shortcut(..), errorName, request)

{-| Ky 2.2.0 (`core/Ky.js`, `core/constants.js`, `utils/normalize.js`, `utils/merge.js`,
`utils/timeout.js`, `utils/is-network-error.js` and `errors/*.js`), the subset in
`libraries/ky/CONTRACT.md`.

Ky's async methods are `Io` actions here, and the request object Ky mutates is a `Ky` record that
each step hands to the next. What Ky throws, they give as `Err`.

-}

import Host exposing (AbortController, Headers, Io(..), JsError, Outcome, Request, Response, Signal, andThen, andThenOk, map)
import Json.Decode as D
import Json.Encode as E
import RetryTiming


{-| Ky's own errors, or what a host API, the transport or one of Ky's option checks threw (a
TypeError, RangeError, SyntaxError or abort reason).
-}
type Failure
    = HttpError { response : Response, request : Request, data : Maybe D.Value }
    | NetworkError { request : Request, cause : JsError }
    | TimeoutError { request : Request }
    | Thrown JsError


{-| `error.name`, as the hooks and the caller read it.
-}
errorName : Failure -> String
errorName failure =
    case failure of
        HttpError _ ->
            "HTTPError"

        NetworkError _ ->
            "NetworkError"

        TimeoutError _ ->
            "TimeoutError"

        Thrown error ->
            error.name


{-| A JS error Ky throws itself, from its option checks.
-}
kyError : String -> String -> Failure
kyError name message =
    Thrown { name = name, message = Just message, isError = True, hasStack = True, sentryCaptured = False }


type alias Hooks s =
    { init : List (Io s ())
    , beforeRequest : List (Request -> Int -> Io s ())
    , afterResponse : List (Request -> Response -> Int -> Io s ())
    , beforeRetry : List (Request -> Failure -> Int -> Io s ())
    , beforeError : List (Request -> Failure -> Int -> Io s ())
    }


{-| The caller's retry options, as the JS values given (Nothing where omitted); Ky's
normalizeRetryOptions validates them.
-}
type alias RetryOptions s =
    { limit : Maybe D.Value
    , methods : Maybe D.Value
    , statusCodes : Maybe D.Value
    , maxRetryAfter : Maybe D.Value
    , backoffLimit : Maybe D.Value
    , retryOnTimeout : Maybe D.Value
    , delay : Int -> Io s Float
    }


type alias Options s =
    { method : Maybe String
    , headers : Maybe D.Value
    , json : Maybe D.Value
    , retry : RetryOptions s
    , timeout : Maybe D.Value
    , throwHttpErrors : Maybe D.Value
    , hooks : Hooks s
    , fetch : Request -> Io s Outcome
    , signal : Maybe Signal
    }


{-| The body shortcuts.
-}
type Shortcut
    = Json
    | Text


mimeType : Shortcut -> String
mimeType shortcut =
    case shortcut of
        Json ->
            "application/json"

        Text ->
            "text/*"


type alias Retry s =
    { limit : Float
    , methods : List String
    , statusCodes : List Float
    , afterStatusCodes : List Float
    , maxRetryAfter : Float
    , backoffLimit : Float
    , delay : Int -> Io s Float
    , retryOnTimeout : Bool
    }


requestMethods : List String
requestMethods =
    [ "get", "post", "put", "patch", "head", "delete", "query" ]


retryMethods : List String
retryMethods =
    [ "get", "put", "head", "delete", "options", "trace", "query" ]


{-| The maximum value of a 32bit int (see issue #117)
-}
maxSafeTimeout : Float
maxSafeTimeout =
    2147483647


maxErrorResponseBodySize : Int
maxErrorResponseBodySize =
    10 * 1024 * 1024


{-| Elm's String.toLower and String.toUpper are JS's toLowerCase and toUpperCase.
-}
normalizeRequestMethod : String -> String
normalizeRequestMethod input =
    if List.member (String.toLower input) requestMethods then
        String.toUpper input

    else
        input


{-| Common methods for retry matching are case-insensitive; custom ones keep their case.
-}
normalizeRetryMethod : String -> String
normalizeRetryMethod method =
    let
        lowercase =
            String.toLower method
    in
    if List.member lowercase (requestMethods ++ [ "options", "trace" ]) then
        lowercase

    else
        method


{-| The value as a JS number, if it is one.
-}
number : D.Value -> Maybe Float
number =
    D.decodeValue D.float >> Result.toMaybe


{-| `Number.isInteger`: elm/json's int decoder accepts exactly the finite integral numbers.
-}
integer : D.Value -> Maybe Float
integer =
    D.decodeValue D.int >> Result.toMaybe >> Maybe.map toFloat


nonNegative : D.Value -> Maybe Float
nonNegative value =
    number value |> Maybe.andThen (\n -> if n >= 0 then Just n else Nothing)


{-| Every value checked, or the first failure.
-}
every : (D.Value -> Maybe a) -> Failure -> List D.Value -> Result Failure (List a)
every check failure values =
    List.foldr (\value rest -> Result.map2 (::) (check value |> Result.fromMaybe failure) rest) (Ok []) values


{-| The defaults for undefined values, and Ky's checks, in Ky's order.
-}
normalizeRetryOptions : RetryOptions s -> Result Failure (Retry s)
normalizeRetryOptions options =
    let
        limit =
            case options.limit of
                Nothing ->
                    Ok 2

                Just value ->
                    integer value
                        |> Maybe.andThen (\n -> if n >= 0 then Just n else Nothing)
                        |> Result.fromMaybe (kyError "TypeError" "`retry.limit` must be a finite, non-negative integer")

        array key value =
            case value of
                Nothing ->
                    Ok Nothing

                Just given ->
                    D.decodeValue (D.list D.value) given
                        |> Result.map Just
                        |> Result.mapError (\_ -> kyError "Error" ("retry." ++ key ++ " must be an array"))

        limitOption key value =
            case value of
                Nothing ->
                    Ok (1 / 0)

                Just given ->
                    nonNegative given
                        |> Result.fromMaybe (kyError "TypeError" ("`retry." ++ key ++ "` must be a non-negative number or `Infinity`"))

        retryOnTimeout =
            case options.retryOnTimeout of
                Nothing ->
                    Ok False

                Just value ->
                    D.decodeValue D.bool value
                        |> Result.mapError (\_ -> kyError "TypeError" "`retry.retryOnTimeout` must be a boolean")
    in
    limit
        |> Result.andThen
            (\validLimit ->
                Result.map2 Tuple.pair (array "methods" options.methods) (array "statusCodes" options.statusCodes)
                    |> Result.andThen
                        (\( methods, statusCodes ) ->
                            -- A mistyped entry only ever fails to match, so it is reported instead.
                            Result.map2 Tuple.pair
                                (methods
                                    |> Maybe.map (every (D.decodeValue D.string >> Result.toMaybe) (kyError "TypeError" "`retry.methods` must only contain strings"))
                                    |> Maybe.withDefault (Ok retryMethods)
                                )
                                (statusCodes
                                    |> Maybe.map (every integer (kyError "TypeError" "`retry.statusCodes` must only contain numbers"))
                                    |> Maybe.withDefault (Ok [ 408, 413, 429, 500, 502, 503, 504 ])
                                )
                        )
                    |> Result.andThen
                        (\( methods, statusCodes ) ->
                            Result.map3
                                (\maxRetryAfter backoffLimit timeoutRetries ->
                                    { limit = validLimit
                                    , methods = List.map normalizeRetryMethod methods
                                    , statusCodes = statusCodes
                                    , afterStatusCodes = [ 413, 429, 503 ]
                                    , maxRetryAfter = maxRetryAfter
                                    , backoffLimit = backoffLimit
                                    , delay = options.delay
                                    , retryOnTimeout = timeoutRetries
                                    }
                                )
                                (limitOption "maxRetryAfter" options.maxRetryAfter)
                                (limitOption "backoffLimit" options.backoffLimit)
                                retryOnTimeout
                        )
            )


{-| Milliseconds, or Nothing for `false`.
-}
validateTimeoutOption : D.Value -> Result Failure (Maybe Float)
validateTimeoutOption value =
    if D.decodeValue D.bool value == Ok False then
        Ok Nothing

    else
        case nonNegative value of
            -- Infinity lands here too, since it is not a usable delay either.
            Just milliseconds ->
                if milliseconds > maxSafeTimeout then
                    Err (kyError "RangeError" "The `timeout` option cannot be greater than 2147483647")

                else
                    Ok (Just milliseconds)

            Nothing ->
                Err (kyError "TypeError" "The `timeout` option must be a non-negative number or `false`")


{-| Option merging (deepMerge, mergeHeaderContainers) keys the headers by their lowercased name,
the last spelling of a name winning at the place of its first. elm/json reads an object's keys in
JS property order.
-}
lowercaseHeaders : D.Value -> List ( String, D.Value )
lowercaseHeaders headers =
    D.decodeValue (D.keyValuePairs D.value) headers
        |> Result.withDefault []
        |> List.foldl
            (\( key, value ) merged ->
                let
                    name =
                        String.toLower key
                in
                if List.any (\( known, _ ) -> known == name) merged then
                    List.map
                        (\( known, old ) ->
                            if known == name then
                                ( known, value )

                            else
                                ( known, old )
                        )
                        merged

                else
                    merged ++ [ ( name, value ) ]
            )
            []


{-| mergeHeaders: each header set on a fresh Headers, which validates it.
-}
mergeHeaders : List ( String, D.Value ) -> Io s (Result Failure Headers)
mergeHeaders headers =
    let
        setAll target entries =
            case entries of
                [] ->
                    Done (Ok target)

                ( name, value ) :: rest ->
                    Host.setHeader target name value
                        |> andThen
                            (\set ->
                                case set of
                                    Ok () ->
                                        setAll target rest

                                    Err error ->
                                        Done (Err (Thrown error))
                            )
    in
    Host.newHeaders |> andThen (\target -> setAll target headers)


networkErrorMessages : List String
networkErrorMessages =
    [ "network error"
    , "NetworkError when attempting to fetch resource."
    , "The Internet connection appears to be offline."
    , "Network request failed"
    , "fetch failed"
    , "terminated"
    , " A network error occurred."
    , "Network connection lost"
    ]


{-| `utils/is-network-error.js`, inlined by Ky from is-network-error 1.3.2.
-}
isRawNetworkError : JsError -> Bool
isRawNetworkError error =
    case error.message of
        Just message ->
            let
                wrapped prefix =
                    String.startsWith prefix message && String.endsWith ")" message
            in
            if not error.isError || error.name /= "TypeError" then
                False

            else if message == "Load failed" || wrapped "Load failed (" then
                -- Safari 17+ has generic message but no stack for network errors
                not error.hasStack || error.sentryCaptured

            else
                String.startsWith "error sending request for url" message
                    || message
                    == "Failed to fetch"
                    || wrapped "Failed to fetch ("
                    || List.member message networkErrorMessages

        Nothing ->
            False


{-| JS whitespace: what `\s` matches (and String.trim, which is JS's own, removes).
-}
isJsWhitespace : Char -> Bool
isJsWhitespace char =
    let
        code =
            Char.toCode char
    in
    List.member code [ 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0xA0, 0x1680, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF ]
        || (code >= 0x2000 && code <= 0x200A)


{-| Matches JSON subtypes like `json`, `problem+json` and `vnd.api+json`, written out rather than
matched with a pattern, as Ky does.
-}
isJsonContentType : String -> Bool
isJsonContentType contentType =
    let
        mime =
            contentType |> String.split ";" |> List.head |> Maybe.withDefault "" |> String.trim |> String.toLower

        length =
            String.length mime
    in
    if not (String.endsWith "json" mime) || length < 5 then
        False

    else
        case String.slice (length - 5) (length - 4) mime of
            "/" ->
                True

            -- The separator is not the slash, so any slash in the type comes before it.
            "." ->
                String.contains "/" mime

            "+" ->
                String.contains "/" mime

            "-" ->
                String.contains "/" mime

            _ ->
                False


{-| The charset parameter as `/;\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))/i` finds it: after the
first `;` where the pattern matches, a quoted value of one or more characters, or else a run of
characters other than `;`, `,` and whitespace. The case of `charset` is ignored for ASCII letters
only, as a non-Unicode JS pattern ignores it. String.trimLeft is JS's trimStart.
-}
charset : String -> Maybe String
charset contentType =
    case String.indexes ";" contentType |> List.head of
        Nothing ->
            Nothing

        Just index ->
            let
                rest =
                    String.dropLeft (index + 1) contentType

                afterName =
                    String.trimLeft rest

                isCharset =
                    String.toList (String.left 7 afterName)
                        |> List.map (\c -> if Char.toCode c >= 0x41 && Char.toCode c <= 0x5A then Char.fromCode (Char.toCode c + 32) else c)
                        |> String.fromList
                        |> (==) "charset"

                value =
                    String.dropLeft 7 afterName |> String.trimLeft |> String.dropLeft 1 |> String.trimLeft

                quoted =
                    if String.startsWith "\"" value then
                        case String.indexes "\"" (String.dropLeft 1 value) |> List.head of
                            Just end ->
                                if end > 0 then
                                    Just (String.slice 1 (end + 1) value)

                                else
                                    Nothing

                            Nothing ->
                                Nothing

                    else
                        Nothing

                bare =
                    String.toList value
                        |> takeWhile (\c -> c /= ';' && c /= ',' && not (isJsWhitespace c))
                        |> String.fromList
            in
            if isCharset && String.startsWith "=" (String.trimLeft (String.dropLeft 7 afterName)) then
                case quoted of
                    Just label ->
                        Just label

                    Nothing ->
                        if bare /= "" then
                            Just bare

                        else
                            charset rest

            else
                charset rest


takeWhile : (a -> Bool) -> List a -> List a
takeWhile keep values =
    case values of
        value :: rest ->
            if keep value then
                value :: takeWhile keep rest

            else
                []

        [] ->
            []


{-| createTextDecoder: the charset's decoder, or UTF-8 for none or an unknown label.
-}
createTextDecoder : String -> Io s Host.TextDecoder
createTextDecoder contentType =
    case charset contentType of
        Just label ->
            Host.labelledDecoder label
                |> andThen
                    (\decoder ->
                        case decoder of
                            Ok known ->
                                Done known

                            Err _ ->
                                Host.utf8Decoder
                    )

        Nothing ->
            Host.utf8Decoder


contentTypeOf : Response -> Io s String
contentTypeOf response =
    Host.getHeader response.headers "content-type" |> map (Maybe.withDefault "")


{-| An error body as text, or Nothing when it cannot be read or is larger than
maxErrorResponseBodySize. JSON uses UTF-8 regardless of the charset parameter (RFC 8259).
-}
readResponseText : Response -> Io s (Maybe String)
readResponseText response =
    if not response.hasBody then
        Host.responseText response |> map Result.toMaybe

    else
        contentTypeOf response
            |> andThen
                (\contentType ->
                    if isJsonContentType contentType then
                        Host.utf8Decoder

                    else
                        createTextDecoder contentType
                )
            |> andThen
                (\decoder ->
                    Host.arrayBuffer response
                        |> andThen
                            (\read ->
                                case read of
                                    Ok bytes ->
                                        if bytes.length > maxErrorResponseBodySize then
                                            Done Nothing

                                        else
                                            Host.decode decoder bytes |> map Just

                                    Err _ ->
                                        Done Nothing
                            )
                )


{-| #getResponseData: the parsed JSON or the text of an error body, or Nothing when it is empty,
unreadable or unparsable. elm/json parses with JSON.parse.
-}
getResponseData : Response -> Io s (Maybe D.Value)
getResponseData response =
    readResponseText response
        |> andThen
            (\read ->
                case read of
                    Nothing ->
                        Done Nothing

                    Just "" ->
                        Done Nothing

                    Just text ->
                        contentTypeOf response
                            |> map
                                (\contentType ->
                                    if isJsonContentType contentType then
                                        D.decodeString D.value text |> Result.toMaybe

                                    else
                                        Just (E.string text)
                                )
            )


{-| The request Ky mutates as it goes, as a record each step hands to the next.
-}
type alias Ky s =
    { request : Request
    , abortController : AbortController

    -- #options.signal: the user's signal and the abort controller's, combined.
    , signal : Signal
    , retryCount : Int
    , originalRequest : Maybe Request
    , userProvidedAbortSignal : Maybe Signal
    , retry : Retry s
    , timeout : Maybe Float
    , throwHttpErrors : Bool
    , hooks : Hooks s
    , fetch : Request -> Io s Outcome
    }


createManagedSignal : Maybe Signal -> AbortController -> Io s Signal
createManagedSignal user controller =
    case user of
        Just signal ->
            Host.anySignal [ signal, controller.signal ]

        Nothing ->
            Done controller.signal


create : String -> Options s -> Io s (Result Failure (Ky s))
create input options =
    let
        headers =
            options.headers |> Maybe.map lowercaseHeaders |> Maybe.withDefault []

        -- Defaults only replace undefined, so a null reaches the validation and is reported.
        throwHttpErrors =
            case options.throwHttpErrors of
                Nothing ->
                    Ok True

                Just value ->
                    D.decodeValue D.bool value
                        |> Result.mapError (\_ -> kyError "TypeError" "The `throwHttpErrors` option must be a boolean or a function")

        validated =
            normalizeRetryOptions options.retry
                |> Result.andThen
                    (\retryOptions ->
                        Result.map2 (\throws timeout -> ( retryOptions, throws, timeout ))
                            throwHttpErrors
                            (Result.andThen validateTimeoutOption (Ok (Maybe.withDefault (E.float 10000) options.timeout)))
                    )

        -- A content-type from options.headers counts as user-provided.
        userProvidedContentType =
            List.any (\( name, _ ) -> name == "content-type") headers

        body =
            options.json |> Maybe.map (E.encode 0)
    in
    mergeHeaders headers
        |> andThenOk
            (\requestHeaders ->
                case validated of
                    Err failure ->
                        Done (Err failure)

                    Ok ( retryOptions, throws, timeout ) ->
                        Host.abortController
                            |> andThen
                                (\controller ->
                                    createManagedSignal options.signal controller
                                        |> andThen
                                            (\signal ->
                                                (if body /= Nothing && not userProvidedContentType then
                                                    Host.setHeader requestHeaders "content-type" (E.string "application/json")

                                                 else
                                                    Done (Ok ())
                                                )
                                                    |> andThenOk (\_ -> Host.newRequest input { method = normalizeRequestMethod (Maybe.withDefault "GET" options.method), headers = requestHeaders, body = body, signal = signal })
                                                    |> map
                                                        (Result.mapError Thrown
                                                            >> Result.map
                                                                (\built ->
                                                                    { request = built
                                                                    , abortController = controller
                                                                    , signal = signal
                                                                    , retryCount = 0
                                                                    , originalRequest = Nothing
                                                                    , userProvidedAbortSignal = options.signal
                                                                    , retry = retryOptions
                                                                    , timeout = timeout
                                                                    , throwHttpErrors = throws
                                                                    , hooks = options.hooks
                                                                    , fetch = options.fetch
                                                                    }
                                                                )
                                                        )
                                            )
                                )
            )


{-| #sentRequest: fetch replaces the request with the clone it prepares for a possible retry, so
the request that was actually sent is reported.
-}
sentRequest : Ky s -> Request
sentRequest ky =
    Maybe.withDefault ky.request ky.originalRequest


{-| Chromium reports a user abort during a fetch as a network error; the abort reason is surfaced
instead.
-}
throwIfAbortedByUser : Ky s -> Io s (Result Failure ())
throwIfAbortedByUser ky =
    case ky.userProvidedAbortSignal of
        Just signal ->
            Host.aborted signal
                |> map
                    (\reason ->
                        case reason of
                            Just error ->
                                Err (Thrown error)

                            Nothing ->
                                Ok ()
                    )

        Nothing ->
            Done (Ok ())


{-| A hook runs inside #raceWithTotalTimeout(), which first throws a user abort.
-}
runHooks : Ky s -> List hook -> (hook -> Io s ()) -> Io s (Result Failure ())
runHooks ky hooks call =
    case hooks of
        [] ->
            Done (Ok ())

        hook :: rest ->
            throwIfAbortedByUser ky
                |> andThenOk (\_ -> call hook |> andThen (\_ -> runHooks ky rest call))


{-| The request was sent and failed: a user abort wins, and a raw network error becomes a
NetworkError.
-}
fetchFailed : Ky s -> Request -> Failure -> Io s ( Ky s, Result Failure Response )
fetchFailed ky sent failure =
    throwIfAbortedByUser ky
        |> map
            (\aborted ->
                case ( aborted, failure ) of
                    ( Err abort, _ ) ->
                        ( ky, Err abort )

                    ( Ok (), Thrown cause ) ->
                        if isRawNetworkError cause then
                            ( ky, Err (NetworkError { request = sent, cause = cause }) )

                        else
                            ( ky, Err failure )

                    ( Ok (), _ ) ->
                        ( ky, Err failure )
            )


fetch : Ky s -> Io s ( Ky s, Result Failure Response )
fetch ky =
    -- Cloning is done here to prepare in advance for retries, and skipped when they are disabled.
    (if ky.retry.limit > 0 then
        Host.cloneRequest ky.request
            |> andThenOk (\clone -> Host.withSignal clone ky.signal)
            |> map (Result.map Just)

     else
        Done (Ok Nothing)
    )
        |> andThen
            (\prepared ->
                case prepared of
                    Err error ->
                        Done ( ky, Err (Thrown error) )

                    Ok retryRequest ->
                        let
                            sent =
                                ky.request

                            next =
                                { ky | originalRequest = Just sent, request = Maybe.withDefault sent retryRequest }
                        in
                        case next.timeout of
                            Nothing ->
                                next.fetch sent |> andThen (settleFetch next sent)

                            -- utils/timeout.js: when the timer fires first, it aborts the
                            -- controller and the attempt fails with a TimeoutError; a response
                            -- arriving later is discarded.
                            Just milliseconds ->
                                Host.timed milliseconds (next.fetch sent)
                                    |> andThen
                                        (\outcome ->
                                            case outcome of
                                                Just settled ->
                                                    settleFetch next sent settled

                                                Nothing ->
                                                    Host.abort next.abortController Nothing
                                                        |> andThen (\_ -> fetchFailed next sent (TimeoutError { request = sent }))
                                        )
            )


settleFetch : Ky s -> Request -> Outcome -> Io s ( Ky s, Result Failure Response )
settleFetch ky sent outcome =
    case outcome of
        Ok response ->
            Done ( ky, Ok response )

        Err error ->
            fetchFailed ky sent (Thrown error)


{-| #calculateDelay(): the retry.delay backoff, capped by retry.backoffLimit.
-}
calculateDelay : Ky s -> Io s (Result Failure (Maybe Float))
calculateDelay ky =
    ky.retry.delay (ky.retryCount + 1)
        |> map
            (\retryDelay ->
                if retryDelay >= 0 then
                    Ok (Just (min ky.retry.backoffLimit retryDelay))

                else
                    Err (kyError "TypeError" "`retry.delay` must return a non-negative number or `Infinity`")
            )


{-| #calculateRetryDelay(): the wait before retrying the error, or Nothing when it is not retried
and fails the call as it is.
-}
calculateRetryDelay : Ky s -> Failure -> Io s (Result Failure (Maybe Float))
calculateRetryDelay ky error =
    if toFloat ky.retryCount >= ky.retry.limit then
        Done (Ok Nothing)

    else if not (List.member (normalizeRetryMethod ky.request.method) ky.retry.methods) then
        Done (Ok Nothing)

    else
        case error of
            TimeoutError _ ->
                if ky.retry.retryOnTimeout then
                    calculateDelay ky

                else
                    Done (Ok Nothing)

            HttpError { response } ->
                let
                    status =
                        toFloat response.status

                    -- Without server-provided timing, a 413 is not retried and the rest back off.
                    otherStatus =
                        if status == 413 then
                            Done (Ok Nothing)

                        else
                            calculateDelay ky
                in
                if not (List.member status ky.retry.statusCodes) then
                    Done (Ok Nothing)

                else
                    RetryTiming.getRetryTimingHeader response.headers
                        |> andThen
                            (\header ->
                                case header of
                                    Just timing ->
                                        if List.member status ky.retry.afterStatusCodes then
                                            RetryTiming.calculateRetryTimingDelay timing
                                                |> andThen
                                                    (\after ->
                                                        case after of
                                                            -- No jitter applies to a server's timing.
                                                            Just delay ->
                                                                Done (Ok (Just (min ky.retry.maxRetryAfter delay)))

                                                            -- Malformed retry timing headers should not disable
                                                            -- retries; they only lose their server-provided timing.
                                                            Nothing ->
                                                                calculateDelay ky
                                                    )

                                        else
                                            otherStatus

                                    Nothing ->
                                        otherStatus
                            )

            NetworkError _ ->
                calculateDelay ky

            -- Only known retriable error types are retried.
            Thrown _ ->
                Done (Ok Nothing)


{-| #retry() and #retryFromError(), which call each other, as in Ky.
-}
retry : Ky s -> Io s ( Ky s, Result Failure Response )
retry ky =
    fetch ky
        |> andThen
            (\( fetched, result ) ->
                case result of
                    Ok response ->
                        Done ( fetched, Ok response )

                    Err error ->
                        retryFromError fetched error
            )


retryFromError : Ky s -> Failure -> Io s ( Ky s, Result Failure Response )
retryFromError ky error =
    throwIfAbortedByUser ky
        |> andThenOk (\_ -> calculateRetryDelay ky error)
        |> andThen
            (\decision ->
                case decision of
                    Err failure ->
                        Done ( ky, Err failure )

                    Ok Nothing ->
                        Done ( ky, Err error )

                    Ok (Just delay) ->
                        Host.sleep (min delay maxSafeTimeout)
                            |> andThen (\_ -> Host.aborted ky.abortController.signal)
                            |> andThen
                                (\timedOut ->
                                    -- Reset the abort controller if it was aborted (on a timeout
                                    -- retry), so hooks and the retried request get a fresh signal.
                                    case timedOut of
                                        Just _ ->
                                            Host.abortController
                                                |> andThen
                                                    (\controller ->
                                                        createManagedSignal ky.userProvidedAbortSignal controller
                                                            |> andThen
                                                                (\signal ->
                                                                    Host.withSignal ky.request signal
                                                                        |> map (Result.map (\resignalled -> { ky | abortController = controller, signal = signal, request = resignalled }))
                                                                )
                                                    )

                                        Nothing ->
                                            Done (Ok ky)
                                )
                            |> andThen
                                (\reset ->
                                    case reset of
                                        Err thrown ->
                                            Done ( ky, Err (Thrown thrown) )

                                        Ok ready ->
                                            runHooks ready ready.hooks.beforeRetry (\hook -> hook ready.request error (ready.retryCount + 1))
                                                |> andThen
                                                    (\hooked ->
                                                        case hooked of
                                                            Err failure ->
                                                                Done ( ready, Err failure )

                                                            Ok () ->
                                                                retry { ready | retryCount = ready.retryCount + 1 }
                                                    )
                                )
            )


{-| #run(): the beforeRequest hooks, the first attempt, and each response in turn until one is
kept.
-}
run : Ky s -> Io s ( Ky s, Result Failure Response )
run ky =
    runHooks ky ky.hooks.beforeRequest (\hook -> hook ky.request 0)
        |> andThen
            (\hooked ->
                case hooked of
                    Err failure ->
                        Done ( ky, Err failure )

                    Ok () ->
                        retry ky |> andThen handleResponse
            )


handleResponse : ( Ky s, Result Failure Response ) -> Io s ( Ky s, Result Failure Response )
handleResponse ( ky, result ) =
    case result of
        Err failure ->
            Done ( ky, Err failure )

        Ok response ->
            runHooks ky ky.hooks.afterResponse (\hook -> hook ky.request response ky.retryCount)
                |> andThen
                    (\hooked ->
                        case hooked of
                            Err failure ->
                                Done ( ky, Err failure )

                            Ok () ->
                                if response.ok || not ky.throwHttpErrors then
                                    Done ( ky, Ok response )

                                else
                                    getResponseData response
                                        |> andThen
                                            (\data ->
                                                let
                                                    error =
                                                        HttpError { response = response, request = sentRequest ky, data = data }
                                                in
                                                throwIfAbortedByUser ky
                                                    |> andThen
                                                        (\aborted ->
                                                            case aborted of
                                                                Err abort ->
                                                                    Done ( ky, Err abort )

                                                                Ok () ->
                                                                    retryFromError ky error |> andThen handleResponse
                                                        )
                                            )
                    )


{-| #throwProcessedError(): the beforeError hooks see the error, which then fails the call.
-}
throwProcessedError : Ky s -> Failure -> Io s (Result Failure a)
throwProcessedError ky error =
    Host.sequence (List.map (\hook -> hook (sentRequest ky) error ky.retryCount) ky.hooks.beforeError)
        |> map (\_ -> Err error)


{-| `ky(input, options)[shortcut]()`: the init hooks, the request, the run and the body read. A
construction error fails without running the beforeError hooks, and so does a body that is not
JSON.
-}
request : String -> Options s -> Shortcut -> Io s (Result Failure D.Value)
request input options shortcut =
    Host.sequence options.hooks.init
        |> andThen (\_ -> create input options)
        |> andThenOk
            (\ky ->
                -- Before dispatch, ky.request is the request that will be sent.
                Host.getHeader ky.request.headers "accept"
                    |> andThen
                        (\accept ->
                            Host.setHeader ky.request.headers
                                "accept"
                                (E.string (accept |> Maybe.andThen (\given -> if given == "" then Nothing else Just given) |> Maybe.withDefault (mimeType shortcut)))
                        )
                    |> andThen
                        (\_ ->
                            run ky
                                |> andThen
                                    (\( finished, result ) ->
                                        case result of
                                            Err failure ->
                                                throwProcessedError finished failure

                                            Ok response ->
                                                Host.responseText response
                                                    |> map
                                                        (\text ->
                                                            case text of
                                                                Err error ->
                                                                    Err (Thrown error)

                                                                Ok body ->
                                                                    case shortcut of
                                                                        Text ->
                                                                            Ok (E.string body)

                                                                        Json ->
                                                                            D.decodeString D.value body
                                                                                |> Result.mapError (\_ -> syntaxError)
                                                        )
                                    )
                        )
            )


{-| What JSON.parse throws for a body that is not JSON (elm/json parses with JSON.parse).
-}
syntaxError : Failure
syntaxError =
    Thrown { name = "SyntaxError", message = Nothing, isError = True, hasStack = True, sentryCaptured = False }
