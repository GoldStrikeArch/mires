module Host exposing
    ( AbortController
    , Bytes
    , Call
    , Headers
    , Io(..)
    , JsError
    , Outcome
    , Request
    , Response
    , Signal
    , TextDecoder
    , abort
    , abortController
    , aborted
    , andThen
    , andThenOk
    , anySignal
    , arrayBuffer
    , cloneRequest
    , decode
    , decodeReply
    , encodeCall
    , getHeader
    , labelledDecoder
    , map
    , modify
    , newHeaders
    , newRequest
    , newResponse
    , now
    , onAbort
    , requestText
    , responseText
    , sequence
    , setHeader
    , sleep
    , timed
    , timerCall
    , utf8Decoder
    , withSignal
    )

{-| The host operations Ky drives (Fetch, AbortController and AbortSignal, TextDecoder, timers and
the clock), as actions of `Io`, the type Ky's Elm code is written in. The JS host (`js/index.js`)
performs each operation and answers with its result or with what it threw; it holds no Ky logic.
Host objects stay in the host, behind handles.

`Io s a` is an action that gives an `a`, with the caller's state `s` (the reference adapter's
trace) threaded through it. It asks the host one operation at a time, and runs the transport
against one timer for Ky's per-attempt timeout (`timed`).

-}

import Json.Decode as D
import Json.Encode as E


{-| A JS error a host operation threw, as Ky reads it: its name and message, whether it is an
Error, whether it has a stack, and whether it carries Sentry's `__sentry_captured__` mark.
-}
type alias JsError =
    { name : String
    , message : Maybe String
    , isError : Bool
    , hasStack : Bool
    , sentryCaptured : Bool
    }


{-| One host operation, by name, with its arguments.
-}
type alias Call =
    { op : String
    , arguments : List ( String, E.Value )
    }


type alias Outcome =
    Result JsError Response


type Io s a
    = Done a
      -- The host answered against the protocol: the port fails fast, and `run` rejects.
    | Fail String
    | Ask Call (Result JsError D.Value -> Io s a)
    | Modify (s -> ( s, Io s a ))
      -- The transport raced against a timer of that many milliseconds: the transport's outcome,
      -- or Nothing when the timer fires first.
    | Timed Float (Io s Outcome) (Maybe Outcome -> Io s a)


andThen : (a -> Io s b) -> Io s a -> Io s b
andThen f io =
    case io of
        Done value ->
            f value

        Fail reason ->
            Fail reason

        Ask call continue ->
            Ask call (continue >> andThen f)

        Modify change ->
            Modify
                (\state ->
                    let
                        ( next, rest ) =
                            change state
                    in
                    ( next, andThen f rest )
                )

        Timed milliseconds transport continue ->
            Timed milliseconds transport (continue >> andThen f)


map : (a -> b) -> Io s a -> Io s b
map f =
    andThen (f >> Done)


{-| Binds the success of a result; a failure ends the chain as it is.
-}
andThenOk : (a -> Io s (Result e b)) -> Io s (Result e a) -> Io s (Result e b)
andThenOk f =
    andThen
        (\result ->
            case result of
                Ok value ->
                    f value

                Err error ->
                    Done (Err error)
        )


{-| Runs the actions one after the other.
-}
sequence : List (Io s ()) -> Io s ()
sequence actions =
    case actions of
        [] ->
            Done ()

        action :: rest ->
            action |> andThen (\_ -> sequence rest)


modify : (s -> ( s, a )) -> Io s a
modify change =
    Modify (\state -> Tuple.mapSecond Done (change state))


{-| A call to the host, as the host reads it: `{ id, op, ...arguments }`.
-}
encodeCall : Int -> Call -> E.Value
encodeCall id call =
    E.object (( "id", E.int id ) :: ( "op", E.string call.op ) :: call.arguments)


{-| The host's answer to a call: `{ id, ok }` with what the operation gave, or `{ id, error }` with
what it threw.
-}
decodeReply : D.Value -> Maybe ( Int, Result JsError D.Value )
decodeReply =
    D.decodeValue
        (D.map2 Tuple.pair
            (D.field "id" D.int)
            (D.oneOf [ D.field "error" jsError |> D.map Err, D.field "ok" D.value |> D.map Ok ])
        )
        >> Result.toMaybe


{-| Asks the host to perform an operation, and decodes its answer: what the operation gave, or
the JS error it threw.
-}
ask : String -> List ( String, E.Value ) -> D.Decoder a -> Io s (Result JsError a)
ask op arguments decoder =
    Ask { op = op, arguments = arguments }
        (\reply ->
            case reply of
                Ok value ->
                    case D.decodeValue decoder value of
                        Ok decoded ->
                            Done (Ok decoded)

                        Err _ ->
                            Fail ("unexpected host answer to " ++ op)

                Err error ->
                    Done (Err error)
        )


{-| An operation that cannot throw.
-}
perform : String -> List ( String, E.Value ) -> D.Decoder a -> Io s a
perform op arguments decoder =
    ask op arguments decoder
        |> andThen
            (\result ->
                case result of
                    Ok value ->
                        Done value

                    Err error ->
                        Fail (op ++ " threw " ++ error.name)
            )


handle : Int -> E.Value
handle =
    E.int


type Signal
    = Signal Int


type Headers
    = Headers Int


type TextDecoder
    = TextDecoder Int


type alias AbortController =
    { handle : Int
    , signal : Signal
    }


type alias Request =
    { handle : Int
    , method : String
    , url : String
    , headers : Headers
    , signal : Signal
    }


type alias Response =
    { handle : Int
    , status : Int
    , ok : Bool
    , headers : Headers
    , hasBody : Bool
    }


type alias Bytes =
    { handle : Int
    , length : Int
    }


abortController : Io s AbortController
abortController =
    perform "abortController"
        []
        (D.map2 AbortController (D.field "handle" D.int) (D.field "signal" (D.map Signal D.int)))


{-| `abort(reason)`: an Error with the given name and message, or the platform's AbortError
DOMException.
-}
abort : AbortController -> Maybe { name : String, message : String } -> Io s ()
abort controller reason =
    perform "abort"
        [ ( "controller", handle controller.handle )
        , ( "reason"
          , reason
                |> Maybe.map (\error -> E.object [ ( "name", E.string error.name ), ( "message", E.string error.message ) ])
                |> Maybe.withDefault E.null
          )
        ]
        (D.succeed ())


{-| `AbortSignal.any`: aborted as soon as one of the signals is, with that one's reason.
-}
anySignal : List Signal -> Io s Signal
anySignal signals =
    perform "anySignal"
        [ ( "signals", E.list (\(Signal id) -> handle id) signals ) ]
        (D.map Signal D.int)


{-| The reason, if the signal has aborted: what `throwIfAborted()` throws.
-}
aborted : Signal -> Io s (Maybe JsError)
aborted (Signal id) =
    perform "aborted" [ ( "signal", handle id ) ] (D.nullable jsError)


{-| Answers once the signal aborts, with its reason.
-}
onAbort : Signal -> Io s JsError
onAbort (Signal id) =
    perform "onAbort" [ ( "signal", handle id ) ] jsError


jsError : D.Decoder JsError
jsError =
    D.map5 JsError
        (D.field "name" D.string)
        (D.field "message" (D.nullable D.string))
        (D.field "isError" D.bool)
        (D.field "hasStack" D.bool)
        (D.field "sentryCaptured" D.bool)


newHeaders : Io s Headers
newHeaders =
    perform "newHeaders" [] (D.map Headers D.int)


{-| `Headers.set`: it validates the name and the value (a TypeError) and stores the value's
ByteString conversion, so any JSON value is accepted as JS accepts it.
-}
setHeader : Headers -> String -> E.Value -> Io s (Result JsError ())
setHeader (Headers id) name value =
    ask "setHeader" [ ( "headers", handle id ), ( "name", E.string name ), ( "value", value ) ] (D.succeed ())


getHeader : Headers -> String -> Io s (Maybe String)
getHeader (Headers id) name =
    perform "getHeader" [ ( "headers", handle id ), ( "name", E.string name ) ] (D.nullable D.string)


request : D.Decoder Request
request =
    D.map5 Request
        (D.field "handle" D.int)
        (D.field "method" D.string)
        (D.field "url" D.string)
        (D.field "headers" (D.map Headers D.int))
        (D.field "signal" (D.map Signal D.int))


{-| `new Request(input, {method, headers, body, signal})`, or the TypeError it throws.
-}
newRequest : String -> { method : String, headers : Headers, body : Maybe String, signal : Signal } -> Io s (Result JsError Request)
newRequest input init =
    let
        (Headers headers) =
            init.headers

        (Signal signal) =
            init.signal
    in
    ask "newRequest"
        [ ( "input", E.string input )
        , ( "method", E.string init.method )
        , ( "headers", handle headers )
        , ( "body", init.body |> Maybe.map E.string |> Maybe.withDefault E.null )
        , ( "signal", handle signal )
        ]
        request


{-| `new Request(request, {signal})`: the same request, following another signal.
-}
withSignal : Request -> Signal -> Io s (Result JsError Request)
withSignal from (Signal signal) =
    ask "withSignal" [ ( "request", handle from.handle ), ( "signal", handle signal ) ] request


cloneRequest : Request -> Io s (Result JsError Request)
cloneRequest from =
    ask "cloneRequest" [ ( "request", handle from.handle ) ] request


requestText : Request -> Io s (Result JsError String)
requestText from =
    ask "requestText" [ ( "request", handle from.handle ) ] D.string


{-| `new Response(body, {status, headers})`, or the RangeError or TypeError it throws.
-}
newResponse : D.Value -> D.Value -> D.Value -> Io s (Result JsError Response)
newResponse body status headers =
    ask "newResponse"
        [ ( "body", body ), ( "status", status ), ( "headers", headers ) ]
        (D.map5 Response
            (D.field "handle" D.int)
            (D.field "status" D.int)
            (D.field "ok" D.bool)
            (D.field "headers" (D.map Headers D.int))
            (D.field "hasBody" D.bool)
        )


responseText : Response -> Io s (Result JsError String)
responseText from =
    ask "responseText" [ ( "response", handle from.handle ) ] D.string


{-| The whole body, as an ArrayBuffer and its length.
-}
arrayBuffer : Response -> Io s (Result JsError Bytes)
arrayBuffer from =
    ask "arrayBuffer" [ ( "response", handle from.handle ) ] (D.map2 Bytes (D.field "handle" D.int) (D.field "length" D.int))


utf8Decoder : Io s TextDecoder
utf8Decoder =
    perform "textDecoder" [ ( "label", E.null ) ] (D.map TextDecoder D.int)


{-| A decoder for an Encoding Standard label, or the RangeError for an unknown one.
-}
labelledDecoder : String -> Io s (Result JsError TextDecoder)
labelledDecoder label =
    ask "textDecoder" [ ( "label", E.string label ) ] (D.map TextDecoder D.int)


decode : TextDecoder -> Bytes -> Io s String
decode (TextDecoder decoder) bytes =
    perform "decode" [ ( "decoder", handle decoder ), ( "bytes", handle bytes.handle ) ] D.string


now : Io s Float
now =
    perform "now" [] D.float


{-| The host's `setTimeout`: it answers once that many milliseconds have passed.
-}
timerCall : Float -> Call
timerCall milliseconds =
    { op = "timer", arguments = [ ( "milliseconds", E.float milliseconds ) ] }


sleep : Float -> Io s ()
sleep milliseconds =
    Ask (timerCall milliseconds) (\_ -> Done ())


{-| The transport raced against a timer: its outcome, or Nothing when the timer fires first.
-}
timed : Float -> Io s Outcome -> Io s (Maybe Outcome)
timed milliseconds transport =
    Timed milliseconds transport Done
