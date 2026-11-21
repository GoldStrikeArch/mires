port module Main exposing (main)

{-| The Elm program: it runs the adapter's `Io` (`Scenario`) against the JS host.

The runtime runs one fiber at a time, as JavaScript runs one job at a time, and keeps a queue of
ready fibers as JavaScript keeps its microtask queue:

  - A fiber runs until it waits: for a host promise, a timer, or a promise of its own. `spawn` runs
    the new fiber first and its caller next, as an async function call does.
  - A settled promise queues its waiters behind the ready fibers, as promise reactions are queued.
  - Every host primitive is one port command, and the host answers it at once, inside the port
    call: with what it gave or threw, or, for a primitive that answers with a promise, that it has
    started. A started primitive's outcome comes later, in the continuation of the host promise
    that settled; its fiber then runs, and every fiber it makes ready runs after it, before the
    host gets control back.
  - `timeout(0)` is `Process.sleep 0`; the runtime goes on with the ready fibers at once, and the
    sleeping fiber is woken by the timer.

Port commands are never batched (Elm delivers a batch's port commands in reverse order).

-}

import Dict exposing (Dict)
import Io exposing (Io(..), JsError, Thrown(..))
import Json.Decode as D
import Json.Encode as E
import Process
import Scenario
import Task
import World exposing (World)


port command : E.Value -> Cmd msg


port receive : (D.Value -> msg) -> Sub msg


type alias Fiber =
    Io World ()


type Slot
    = Pending (List (Result Thrown E.Value -> Fiber))
    | Settled (Result Thrown E.Value)


{-| What the command in flight is waiting for: its answer, or the news that its promise started.
-}
type Ack
    = Answer (Result JsError D.Value -> Fiber)
    | Started


type alias Model =
    { world : World
    , front : List Fiber
    , back : List Fiber
    , promises : Dict Int Slot
    , nextPromise : Int
    , nextCommand : Int
    , acking : Maybe ( Int, Ack )
    , hostWaiting : Dict Int (Result JsError D.Value -> Fiber)
    , timers : Dict Int Fiber
    , nextTimer : Int
    }


type Msg
    = Receive D.Value
    | Resume
    | Woke Int


main : Program () Model Msg
main =
    Platform.worker
        { init = \() -> ( initial, Cmd.none )
        , update = update
        , subscriptions = \_ -> receive Receive
        }


initial : Model
initial =
    { world =
        World.initial { href = "", origin = "" }
            { base = "", prefix = "", source = "alpha", traces = [], logs = Dict.empty, events = [] }
    , front = []
    , back = []
    , promises = Dict.empty
    , nextPromise = 0
    , nextCommand = 0
    , acking = Nothing
    , hostWaiting = Dict.empty
    , timers = Dict.empty
    , nextTimer = 0
    }


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        Receive message ->
            receiveFromHost message model

        Resume ->
            run (Done ()) model

        Woke timer ->
            case Dict.get timer model.timers of
                Just fiber ->
                    run fiber { model | timers = Dict.remove timer model.timers }

                Nothing ->
                    ( model, Cmd.none )


jsError : D.Decoder JsError
jsError =
    D.map4 JsError (D.field "name" D.string) (D.field "message" D.string) (D.field "isError" D.bool) (D.field "text" D.string)


receiveFromHost : D.Value -> Model -> ( Model, Cmd Msg )
receiveFromHost message model =
    let
        kind =
            D.decodeValue (D.field "kind" D.string) message |> Result.withDefault ""

        id =
            D.decodeValue (D.field "id" D.int) message |> Result.withDefault -1

        outcome =
            case kind of
                "threw" ->
                    D.decodeValue (D.field "error" jsError) message |> Result.withDefault (JsError "Error" "unreadable host error" True "Error") |> Err

                "rejected" ->
                    D.decodeValue (D.field "error" jsError) message |> Result.withDefault (JsError "Error" "unreadable host error" True "Error") |> Err

                _ ->
                    D.decodeValue (D.field "value" D.value) message |> Result.withDefault E.null |> Ok
    in
    case ( kind, model.acking ) of
        ( "start", _ ) ->
            case D.decodeValue Scenario.startDecoder message of
                Ok start ->
                    run (Scenario.run start |> Io.attempt |> Io.andThen (Result.mapError Io.nameOf >> Exit)) model

                Err _ ->
                    ( model, exitCommand model.nextCommand (Err "unreadable input") )

        ( "started", Just ( awaited, Started ) ) ->
            if awaited == id then
                run (Done ()) { model | acking = Nothing }

            else
                ( model, Cmd.none )

        ( "resolved", _ ) ->
            resumeHost id outcome model

        ( "rejected", _ ) ->
            resumeHost id outcome model

        ( _, Just ( awaited, Answer continue ) ) ->
            if awaited == id then
                run (continue outcome) { model | acking = Nothing }

            else
                ( model, Cmd.none )

        _ ->
            ( model, Cmd.none )


resumeHost : Int -> Result JsError D.Value -> Model -> ( Model, Cmd Msg )
resumeHost id outcome model =
    case Dict.get id model.hostWaiting of
        Just continue ->
            run (continue outcome) { model | hostWaiting = Dict.remove id model.hostWaiting }

        Nothing ->
            ( model, Cmd.none )


hostCommand : Int -> String -> List ( String, E.Value ) -> Cmd Msg
hostCommand id op arguments =
    command (E.object (( "id", E.int id ) :: ( "op", E.string op ) :: arguments))


exitCommand : Int -> Result String E.Value -> Cmd Msg
exitCommand id outcome =
    case outcome of
        Ok output ->
            hostCommand id "exit" [ ( "output", output ) ]

        Err reason ->
            hostCommand id "exit" [ ( "failed", E.string reason ) ]


enqueue : Fiber -> Model -> Model
enqueue fiber model =
    { model | back = fiber :: model.back }


{-| A spawned fiber: the computation, then the settling of its promise with its outcome.

This closure is built outside `run`. Elm compiles the self-recursive `run` to a loop whose pattern
variables are shared by every branch and iteration, so a closure made inside it and called later
would see the `promise` of whatever branch ran last.

-}
settling : Int -> Io World E.Value -> Fiber
settling promise child =
    Io.attempt child |> Io.andThen (\outcome -> Settle promise outcome (Done ()))


{-| Runs a fiber until it waits or ends, then the next ready fiber; answers the one command the
host must perform next, if any.
-}
run : Fiber -> Model -> ( Model, Cmd Msg )
run io model =
    case io of
        Done () ->
            case model.front of
                fiber :: rest ->
                    run fiber { model | front = rest }

                [] ->
                    case List.reverse model.back of
                        fiber :: rest ->
                            run fiber { model | front = rest, back = [] }

                        [] ->
                            ( model, Cmd.none )

        Throw thrown ->
            ( model, exitCommand model.nextCommand (Err ("uncaught " ++ Io.nameOf thrown)) )

        Call op arguments continue ->
            ( { model | nextCommand = model.nextCommand + 1, acking = Just ( model.nextCommand, Answer continue ) }
            , hostCommand model.nextCommand op arguments
            )

        Start op arguments continue ->
            ( { model
                | nextCommand = model.nextCommand + 1
                , acking = Just ( model.nextCommand, Started )
                , hostWaiting = Dict.insert model.nextCommand continue model.hostWaiting
              }
            , hostCommand model.nextCommand op arguments
            )

        Sleep rest ->
            -- Elm hands a batch to the Task manager in reverse: the timer starts, then the ready
            -- fibers resume, both before the host gets control back.
            ( { model | timers = Dict.insert model.nextTimer rest model.timers, nextTimer = model.nextTimer + 1 }
            , Cmd.batch [ Task.perform (\() -> Resume) (Task.succeed ()), Task.perform (\() -> Woke model.nextTimer) (Process.sleep 0) ]
            )

        Spawn child continue ->
            let
                promise =
                    model.nextPromise
            in
            run (settling promise child)
                { model
                    | nextPromise = promise + 1
                    , promises = Dict.insert promise (Pending []) model.promises
                    , front = continue promise :: model.front
                }

        Await promise continue ->
            case Dict.get promise model.promises of
                Just (Settled outcome) ->
                    run (Done ()) (enqueue (continue outcome) model)

                Just (Pending waiters) ->
                    run (Done ()) { model | promises = Dict.insert promise (Pending (waiters ++ [ continue ])) model.promises }

                Nothing ->
                    ( model, exitCommand model.nextCommand (Err "a wait on an unknown promise") )

        Settle promise outcome rest ->
            case Dict.get promise model.promises of
                Just (Pending waiters) ->
                    run rest (List.foldl (\waiter queued -> enqueue (waiter outcome) queued) { model | promises = Dict.insert promise (Settled outcome) model.promises } waiters)

                _ ->
                    run rest model

        Deferred continue ->
            run (continue model.nextPromise) { model | nextPromise = model.nextPromise + 1, promises = Dict.insert model.nextPromise (Pending []) model.promises }

        Modify change ->
            let
                ( world, rest ) =
                    change model.world
            in
            run rest { model | world = world }

        Exit outcome ->
            -- The host answers the exit at once; fibers still running in the background (a
            -- service worker's cachePut) go on.
            ( { model | nextCommand = model.nextCommand + 1, acking = Just ( model.nextCommand, Answer (\_ -> Done ()) ) }
            , exitCommand model.nextCommand outcome
            )
