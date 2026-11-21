port module Main exposing (main)

{-| Entry of the Elm port: what `libraries/ky/reference.mjs` does, on the port's Ky, and the
interpreter that runs Ky's `Io` actions against the JS host.

The boundary: the host sends the corpus input through `start`. Each request case is read in
place with elm/json, field by field; the option values Ky validates stay the JS values they are
until Ky's own checks decode them. The program asks the host for each Fetch, abort, decoding and
timer operation through `command`, and the host answers through `reply`. The result, one
`[trace, outcome]` per case, goes back through `command` as `{ done }`.

-}

import Host exposing (AbortController, Io(..), JsError, Outcome, Request, andThen, andThenOk, map)
import Json.Decode as D
import Json.Encode as E
import Ky


port start : (D.Value -> msg) -> Sub msg


port reply : (D.Value -> msg) -> Sub msg


port command : E.Value -> Cmd msg


{-| The reference adapter's state for one request case: the trace its hooks record, and how many
replies the transport has given.
-}
type alias Fixture =
    { trace : List E.Value
    , sent : Int
    }


record : List E.Value -> Io Fixture ()
record entry =
    Host.modify (\fixture -> ( { fixture | trace = E.list identity entry :: fixture.trace }, () ))


{-| What the adapter aborts its controller with.
-}
cancel : AbortController -> Io s ()
cancel controller =
    Host.abort controller (Just { name = "AbortError", message = "cancelled" })


{-| A property that is not undefined, which is how Ky tells an option given from one omitted.
elm/json reads a property holding undefined as present, so the value must be one of JSON's types.
-}
present : String -> D.Value -> Maybe D.Value
present key value =
    D.decodeValue (D.field key defined) value |> Result.toMaybe


defined : D.Decoder D.Value
defined =
    D.oneOf
        [ D.null ()
        , D.map (\_ -> ()) D.bool
        , D.map (\_ -> ()) D.float
        , D.map (\_ -> ()) D.string
        , D.map (\_ -> ()) (D.list D.value)
        , D.map (\_ -> ()) (D.keyValuePairs D.value)
        ]
        |> D.andThen (\_ -> D.value)


text : String -> D.Value -> String
text key value =
    D.decodeValue (D.field key D.string) value |> Result.withDefault ""


property : String -> D.Value -> D.Value
property key value =
    present key value |> Maybe.withDefault E.null


nullable : Maybe String -> E.Value
nullable =
    Maybe.map E.string >> Maybe.withDefault E.null


{-| The TypeError the transport throws for a network reply.
-}
failedToFetch : JsError
failedToFetch =
    { name = "TypeError", message = Just "Failed to fetch", isError = True, hasStack = True, sentryCaptured = False }


{-| The injected transport: it records what it was sent and answers with the next reply.
-}
transport : D.Value -> AbortController -> Request -> Io Fixture Outcome
transport spec controller sent =
    Host.requestText sent
        |> andThenOk
            (\body ->
                Host.getHeader sent.headers "accept"
                    |> andThen
                        (\accept ->
                            Host.getHeader sent.headers "content-type"
                                |> andThen
                                    (\contentType ->
                                        Host.getHeader sent.headers "x-matrix"
                                            |> andThen
                                                (\matrix ->
                                                    record
                                                        [ E.string "fetch"
                                                        , E.string sent.method
                                                        , E.string sent.url
                                                        , nullable accept
                                                        , nullable contentType
                                                        , nullable matrix
                                                        , E.string body
                                                        ]
                                                )
                                    )
                        )
                    |> andThen (\_ -> abortAt "fetch" spec controller)
                    |> andThen (\_ -> Host.aborted sent.signal)
                    |> andThen
                        (\reason ->
                            case reason of
                                Just thrown ->
                                    Done (Err thrown)

                                Nothing ->
                                    Host.modify (\fixture -> ( { fixture | sent = fixture.sent + 1 }, fixture.sent ))
                                        |> andThen (\index -> respond (nextReply index spec) sent)
                        )
            )


nextReply : Int -> D.Value -> D.Value
nextReply index spec =
    let
        replies =
            D.decodeValue (D.field "replies" (D.list D.value)) spec |> Result.withDefault []
    in
    replies |> List.drop (min index (List.length replies - 1)) |> List.head |> Maybe.withDefault E.null


respond : D.Value -> Request -> Io Fixture Outcome
respond given sent =
    case text "kind" given of
        "network" ->
            Done (Err failedToFetch)

        "stall" ->
            Host.onAbort sent.signal |> map Err

        _ ->
            Host.newResponse (property "body" given) (property "status" given) (property "headers" given)


abortAt : String -> D.Value -> AbortController -> Io s ()
abortAt point spec controller =
    if text "abort" spec == point then
        cancel controller

    else
        Done ()


{-| The x-matrix header the hooks set: a valid name and value, so it cannot fail.
-}
mark : Request -> String -> Io s ()
mark sent label =
    Host.setHeader sent.headers "x-matrix" (E.string label) |> map (\_ -> ())


delay : Int -> Io Fixture Float
delay attempt =
    let
        milliseconds =
            300 * 2 ^ toFloat (attempt - 1)
    in
    record [ E.string "backoff", E.int attempt, E.float milliseconds ] |> map (\_ -> milliseconds)


hooks : D.Value -> AbortController -> Ky.Hooks Fixture
hooks spec controller =
    { init = [ record [ E.string "init" ] ]
    , beforeRequest =
        [ \sent retryCount ->
            record [ E.string "beforeRequest", E.int retryCount ] |> andThen (\_ -> mark sent "initial")
        ]
    , afterResponse =
        [ \_ response retryCount ->
            record [ E.string "afterResponse", E.int retryCount, E.int response.status ]
        ]
    , beforeRetry =
        [ \sent error retryCount ->
            record [ E.string "beforeRetry", E.int retryCount, E.string (Ky.errorName error) ]
                |> andThen (\_ -> mark sent ("retry-" ++ String.fromInt retryCount))
                |> andThen (\_ -> abortAt "retry" spec controller)
        ]
    , beforeError =
        [ \_ error retryCount ->
            record [ E.string "beforeError", E.int retryCount, E.string (Ky.errorName error) ]
        ]
    }


outcome : Result Ky.Failure D.Value -> E.Value
outcome result =
    case result of
        Ok value ->
            E.list identity [ E.string "ok", value ]

        Err (Ky.HttpError { response, data }) ->
            E.list identity [ E.string "error", E.string "HTTPError", E.int response.status, Maybe.withDefault E.null data ]

        Err failure ->
            E.list identity [ E.string "error", E.string (Ky.errorName failure), E.null, E.null ]


requestCase : D.Value -> Io Fixture E.Value
requestCase spec =
    Host.modify (\_ -> ( { trace = [], sent = 0 }, () ))
        |> andThen (\_ -> Host.abortController)
        |> andThen
            (\controller ->
                abortAt "before" spec controller
                    |> andThen
                        (\_ ->
                            Ky.request (text "url" spec)
                                { method = present "method" spec |> Maybe.andThen (D.decodeValue D.string >> Result.toMaybe)
                                , headers = present "headers" spec
                                , json = present "json" spec
                                , retry =
                                    { limit = present "limit" spec
                                    , methods = present "methods" spec
                                    , statusCodes = present "statusCodes" spec
                                    , maxRetryAfter = present "maxRetryAfter" spec
                                    , backoffLimit = present "backoffLimit" spec
                                    , retryOnTimeout = present "retryOnTimeout" spec
                                    , delay = delay
                                    }
                                , timeout = present "timeout" spec
                                , throwHttpErrors = present "throwHttpErrors" spec
                                , hooks = hooks spec controller
                                , fetch = transport spec controller
                                , signal = Just controller.signal
                                }
                                (if text "read" spec == "json" then
                                    Ky.Json

                                 else
                                    Ky.Text
                                )
                        )
                    |> andThen
                        (\result ->
                            Host.modify
                                (\fixture ->
                                    ( fixture, E.list identity [ E.list identity (List.reverse fixture.trace), outcome result ] )
                                )
                        )
            )


program : D.Value -> Io Fixture E.Value
program input =
    let
        each cases results =
            case cases of
                [] ->
                    Done (E.list identity (List.reverse results))

                spec :: rest ->
                    requestCase spec |> andThen (\result -> each rest (result :: results))
    in
    each (D.decodeValue (D.field "cases" (D.list D.value)) input |> Result.withDefault []) []



-- THE INTERPRETER


{-| What the program is waiting for: one answer, or, during a timed attempt, the transport's next
answer or the timer, whichever comes first.
-}
type Running
    = Idle
    | Waiting Int (Result JsError D.Value -> Io Fixture E.Value)
    | Racing
        { timer : Int
        , pending : Int
        , continue : Result JsError D.Value -> Io Fixture Outcome
        , resume : Maybe Outcome -> Io Fixture E.Value
        }


type alias Model =
    { fixture : Fixture
    , running : Running
    , nextId : Int
    }


type Msg
    = Start D.Value
    | Reply D.Value


{-| Runs the program until it waits on the host, and gives the messages for the host.
-}
runProgram : Io Fixture E.Value -> Model -> ( Model, List E.Value )
runProgram io model =
    case io of
        Done result ->
            ( { model | running = Idle }, [ E.object [ ( "done", result ) ] ] )

        Fail reason ->
            ( { model | running = Idle }, [ E.object [ ( "failed", E.string reason ) ] ] )

        Ask call continue ->
            ( { model | running = Waiting model.nextId continue, nextId = model.nextId + 1 }
            , [ Host.encodeCall model.nextId call ]
            )

        Modify change ->
            let
                ( fixture, next ) =
                    change model.fixture
            in
            runProgram next { model | fixture = fixture }

        Timed milliseconds attempt resume ->
            let
                ( racing, messages ) =
                    runTransport attempt model.nextId resume { model | nextId = model.nextId + 1 }
            in
            ( racing, Host.encodeCall model.nextId (Host.timerCall milliseconds) :: messages )


{-| Runs a timed transport until it waits on the host. When it settles before its timer, the timer
is cleared and the program goes on with its outcome.
-}
runTransport : Io Fixture Outcome -> Int -> (Maybe Outcome -> Io Fixture E.Value) -> Model -> ( Model, List E.Value )
runTransport io timer resume model =
    case io of
        Done settled ->
            runProgram (resume (Just settled)) model
                |> Tuple.mapSecond ((::) (E.object [ ( "clear", E.int timer ) ]))

        Fail reason ->
            runProgram (Fail reason) model

        Ask call continue ->
            ( { model | running = Racing { timer = timer, pending = model.nextId, continue = continue, resume = resume }, nextId = model.nextId + 1 }
            , [ Host.encodeCall model.nextId call ]
            )

        Modify change ->
            let
                ( fixture, next ) =
                    change model.fixture
            in
            runTransport next timer resume { model | fixture = fixture }

        -- A transport does not time itself; a nested race would run without its timer.
        Timed _ inner continue ->
            runTransport (inner |> andThen (Just >> continue)) timer resume model


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        Start input ->
            send (runProgram (program input) { fixture = { trace = [], sent = 0 }, running = Idle, nextId = 1 })

        Reply value ->
            case ( Host.decodeReply value, model.running ) of
                ( Just ( id, answer ), Waiting expected continue ) ->
                    if id == expected then
                        send (runProgram (continue answer) model)

                    else
                        ( model, Cmd.none )

                ( Just ( id, answer ), Racing race ) ->
                    if id == race.timer then
                        -- The timer fired first: the transport is abandoned, and a late answer to
                        -- it is dropped.
                        send (runProgram (race.resume Nothing) model)

                    else if id == race.pending then
                        send (runTransport (race.continue answer) race.timer race.resume model)

                    else
                        ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )


send : ( Model, List E.Value ) -> ( Model, Cmd Msg )
send ( model, messages ) =
    ( model, Cmd.batch (List.map command messages) )


main : Program () Model Msg
main =
    Platform.worker
        { init = \_ -> ( { fixture = { trace = [], sent = 0 }, running = Idle, nextId = 1 }, Cmd.none )
        , update = update
        , subscriptions = \_ -> Sub.batch [ start Start, reply Reply ]
        }
