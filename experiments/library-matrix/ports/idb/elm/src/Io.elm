module Io exposing
    ( Io(..), Reply, Settled, Event
    , succeed, map, andThen, sequence
    , call, await, settle, listen, unlisten, log
    , objectResult, field
    )

{-| What the port asks of its JS host, as a value: a computation that calls host primitives,
waits for promises and listens to host events. `Main` runs it (one fiber at a time, one host
command in flight), and `Idb` builds idb's layers out of it.

Every host primitive is answered synchronously, before the host returns, with what the call gave
or threw (CONTRACT.md, "Elm: the I/O boundary"), so `Call` continues at once. Events (a request's
`success`, a transaction's `complete`, an open's `upgradeneeded`, ...) come later as messages with
the id of the command that set them up; `Listen` routes them to a handler. A promise is an id
too: `Settle` completes it once, and `Await` waits for it, as JS promises do.

-}

import Json.Decode as D
import Json.Encode as E


{-| Every effect holds the rest of the computation as a function, so `andThen` never walks more
than one constructor, however long a run of logs or settlements is.
-}
type Io a
    = Done a
    | Call String (List ( String, E.Value )) (Reply -> Io a)
    | Await Int (Settled -> Io a)
    | Settle Int Settled (() -> Io a)
    | Listen Int (Event -> Io ()) (() -> Io a)
    | Unlisten Int (() -> Io a)
    | Log E.Value (() -> Io a)
    | Finish (List E.Value -> E.Value)
    | Fail String


{-| The answer to a host call: its command id, and what the call returned (`Nothing` for
undefined) or the name of what it threw.
-}
type alias Reply =
    { id : Int, result : Result String (Maybe D.Value) }


{-| A settled promise: its value (`Nothing` for undefined) or the name of its error.
-}
type alias Settled =
    Result String (Maybe D.Value)


{-| A host event: its kind and the whole message.
-}
type alias Event =
    { kind : String, message : D.Value }


succeed : a -> Io a
succeed =
    Done


andThen : (a -> Io b) -> Io a -> Io b
andThen next io =
    case io of
        Done value ->
            next value

        Call op fields continue ->
            Call op fields (continue >> andThen next)

        Await id continue ->
            Await id (continue >> andThen next)

        Settle id settled rest ->
            Settle id settled (rest >> andThen next)

        Listen id handler rest ->
            Listen id handler (rest >> andThen next)

        Unlisten id rest ->
            Unlisten id (rest >> andThen next)

        Log entry rest ->
            Log entry (rest >> andThen next)

        Finish output ->
            Finish output

        Fail name ->
            Fail name


map : (a -> b) -> Io a -> Io b
map f =
    andThen (f >> Done)


{-| Runs `each` on the items one after another, collecting the results. An item that is done at
once is collected in the loop.
-}
sequence : (a -> Io b) -> List a -> Io (List b)
sequence each =
    let
        go done items =
            case items of
                [] ->
                    Done (List.reverse done)

                item :: rest ->
                    case each item of
                        Done result ->
                            go (result :: done) rest

                        io ->
                            io |> andThen (\result -> go (result :: done) rest)
    in
    go []


{-| One host primitive: `op` with its fields; the runtime adds the command id.
-}
call : String -> List ( String, E.Value ) -> Io Reply
call op fields =
    Call op fields Done


await : Int -> Io Settled
await id =
    Await id Done


settle : Int -> Settled -> Io ()
settle id settled =
    Settle id settled (\() -> Done ())


listen : Int -> (Event -> Io ()) -> Io ()
listen id handler =
    Listen id handler (\() -> Done ())


unlisten : Int -> Io ()
unlisten id =
    Unlisten id (\() -> Done ())


log : E.Value -> Io ()
log entry =
    Log entry (\() -> Done ())


{-| A host call's result that is an object (a store, a transaction, a key range...), or what it
threw.
-}
objectResult : Reply -> Result String D.Value
objectResult reply =
    Result.map (Maybe.withDefault E.null) reply.result


{-| A field of a host message, as the host value it is.
-}
field : String -> D.Value -> D.Value
field name message =
    D.decodeValue (D.field name D.value) message |> Result.withDefault E.null
