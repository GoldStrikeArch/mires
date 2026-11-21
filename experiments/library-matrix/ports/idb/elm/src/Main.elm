port module Main exposing (main)

{-| The Elm program: it decodes the script, plays it through the port's idb (`Session`), and runs
the `Io` that does so against the JS host.

The runtime runs one fiber at a time, as JS runs one job at a time. A fiber runs until it sends a
host command, waits for a promise, or ends. The host answers every command synchronously, inside
the port call, so the fiber that sent it resumes at once; there is never more than one command in
flight, and port commands are never batched (Elm delivers a batch's port commands in reverse
order). A settled promise queues its waiters behind the running fiber, as promise reactions are
queued as microtasks; a host event starts its listener as a new fiber. Everything that follows an
event, up to the next wait, therefore runs inside the event's dispatch, while IndexedDB keeps the
transaction active.

-}

import Dict exposing (Dict)
import Io exposing (Io(..))
import Json.Decode as D
import Json.Encode as E
import Platform
import Script
import Session


port command : E.Value -> Cmd msg


port receive : (D.Value -> msg) -> Sub msg


type alias Model =
    { script : Io ()
    , log : List E.Value
    , nextId : Int
    , acking : Maybe ( Int, Io.Reply -> Io () )
    , ready : List (Io ())
    , waiting : Dict Int (List (Io.Settled -> Io ()))
    , settled : Dict Int Io.Settled
    , listeners : Dict Int (Io.Event -> Io ())
    }


type Msg
    = Receive D.Value


main : Program D.Value Model Msg
main =
    Platform.worker
        { init = init
        , update = update
        , subscriptions = \_ -> receive Receive
        }


{-| The flags are the corpus input and a fresh UUID for the database names. An input outside the
contract rejects the call with a TypeError before any database is opened.
-}
init : D.Value -> ( Model, Cmd Msg )
init flags =
    let
        script =
            case ( D.decodeValue (D.field "input" Script.decoder) flags, D.decodeValue (D.field "uuid" D.string) flags ) of
                ( Ok steps, Ok uuid ) ->
                    Session.run uuid steps

                _ ->
                    Fail "TypeError"
    in
    ( { script = script
      , log = []
      , nextId = 0
      , acking = Nothing
      , ready = []
      , waiting = Dict.empty
      , settled = Dict.empty
      , listeners = Dict.empty
      }
    , Cmd.none
    )


update : Msg -> Model -> ( Model, Cmd Msg )
update (Receive message) model =
    let
        text name =
            D.decodeValue (D.field name D.string) message |> Result.withDefault ""

        id =
            D.decodeValue (D.field "id" D.int) message |> Result.withDefault -1
    in
    case text "kind" of
        "start" ->
            run model.script model

        "value" ->
            resume id (Ok (D.decodeValue (D.maybe (D.field "result" D.value)) message |> Result.withDefault Nothing)) model

        "threw" ->
            resume id (Err (text "name")) model

        kind ->
            case Dict.get id model.listeners of
                Just listener ->
                    run (listener { kind = kind, message = message }) model

                Nothing ->
                    ( model, Cmd.none )


{-| The answer to the command in flight resumes the fiber that sent it.
-}
resume : Int -> Result String (Maybe D.Value) -> Model -> ( Model, Cmd Msg )
resume id result model =
    case model.acking of
        Just ( awaited, continue ) ->
            if awaited == id then
                run (continue { id = id, result = result }) { model | acking = Nothing }

            else
                ( model, Cmd.none )

        Nothing ->
            ( model, Cmd.none )


{-| Runs a fiber until it sends a command, waits or ends; then the next ready fiber.
-}
run : Io () -> Model -> ( Model, Cmd Msg )
run io model =
    case io of
        Done () ->
            next model

        Log entry rest ->
            run (rest ()) { model | log = entry :: model.log }

        Call op fields continue ->
            ( { model | nextId = model.nextId + 1, acking = Just ( model.nextId, continue ) }
            , command (E.object (( "op", E.string op ) :: ( "id", E.int model.nextId ) :: fields))
            )

        Await id continue ->
            case Dict.get id model.settled of
                Just settled ->
                    run (continue settled) model

                Nothing ->
                    next { model | waiting = Dict.update id (\waiters -> Just (Maybe.withDefault [] waiters ++ [ continue ])) model.waiting }

        Settle id settled rest ->
            if Dict.member id model.settled then
                run (rest ()) model

            else
                let
                    woken =
                        Dict.get id model.waiting |> Maybe.withDefault [] |> List.map (\continue -> continue settled)
                in
                run (rest ()) { model | settled = Dict.insert id settled model.settled, waiting = Dict.remove id model.waiting, ready = model.ready ++ woken }

        Listen id listener rest ->
            run (rest ()) { model | listeners = Dict.insert id listener model.listeners }

        Unlisten id rest ->
            run (rest ()) { model | listeners = Dict.remove id model.listeners }

        Finish output ->
            ( model, command (E.object [ ( "op", E.string "finish" ), ( "output", output (List.reverse model.log) ) ]) )

        Fail name ->
            ( model, command (E.object [ ( "op", E.string "fail" ), ( "name", E.string name ) ]) )


next : Model -> ( Model, Cmd Msg )
next model =
    case model.ready of
        fiber :: rest ->
            run fiber { model | ready = rest }

        [] ->
            ( model, Cmd.none )
