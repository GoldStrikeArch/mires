module Io exposing
    ( Io(..), Thrown(..), JsError, Promise, promiseId
    , succeed, throw, map, andThen, sequence, forEach, foldl, attempt, catch, finally
    , modify, get, spawn, await, settle, deferred, sleep
    , call, start, exit
    , isError, nameOf, workboxMessage, asDetail
    )

{-| What the port's Workbox is written in: a computation over a world `w` that calls host
primitives, starts concurrent computations and waits for them, as JavaScript async functions do.
`Main` runs it against the JS host, one fiber at a time, with promise reactions queued as
microtasks are; the Workbox modules build the library's layers out of it.

A `Promise a` is eager and has any number of awaiters, as a JavaScript promise: `spawn` starts a
fiber at once and runs it up to its first wait before the caller continues, as an async function
call does. Its value travels as JSON, with the decoder that reads it back.

-}

import Json.Decode as D
import Json.Encode as E


type Io w a
    = Done a
    | Throw Thrown
      -- A host primitive that answers at once, or the failure it threw.
    | Call String (List ( String, E.Value )) (Result JsError D.Value -> Io w a)
      -- A host primitive whose promise settles later.
    | Start String (List ( String, E.Value )) (Result JsError D.Value -> Io w a)
      -- workbox-core's `timeout(0)`: Elm's `Process.sleep 0`, one timer task.
    | Sleep (Io w a)
    | Spawn (Io w E.Value) (Int -> Io w a)
    | Await Int (Result Thrown E.Value -> Io w a)
    | Settle Int (Result Thrown E.Value) (Io w a)
    | Deferred (Int -> Io w a)
    | Modify (w -> ( w, Io w a ))
      -- The end of the program: its output, or the reason the port cannot go on.
    | Exit (Result String E.Value)


{-| A thrown value: a `WorkboxError` (an Error whose `name` is its code), or what a host primitive
or a plugin threw. `details` is the details object as `JSON.stringify` prints it.
-}
type Thrown
    = WorkboxError String (Maybe E.Value)
    | HostError JsError


{-| A JS value that was thrown: its `name` and `message` when it is an Error, whether it is one
(`instanceof Error`), and `String(value)`.
-}
type alias JsError =
    { name : String
    , message : String
    , isError : Bool
    , text : String
    }


type Promise a
    = Promise Int (D.Decoder a)


promiseId : Promise a -> Int
promiseId (Promise id _) =
    id


succeed : a -> Io w a
succeed =
    Done


throw : Thrown -> Io w a
throw =
    Throw


andThen : (a -> Io w b) -> Io w a -> Io w b
andThen next io =
    case io of
        Done value ->
            next value

        Throw thrown ->
            Throw thrown

        Call op arguments continue ->
            Call op arguments (continue >> andThen next)

        Start op arguments continue ->
            Start op arguments (continue >> andThen next)

        Sleep rest ->
            Sleep (andThen next rest)

        Spawn child continue ->
            Spawn child (continue >> andThen next)

        Await id continue ->
            Await id (continue >> andThen next)

        Settle id outcome rest ->
            Settle id outcome (andThen next rest)

        Deferred continue ->
            Deferred (continue >> andThen next)

        Modify change ->
            Modify (change >> Tuple.mapSecond (andThen next))

        Exit outcome ->
            Exit outcome


map : (a -> b) -> Io w a -> Io w b
map f =
    andThen (f >> Done)


{-| `try { io } catch (thrown) { handler(thrown) }`. A spawned fiber's failure is its own.
-}
catch : (Thrown -> Io w a) -> Io w a -> Io w a
catch handler io =
    case io of
        Done value ->
            Done value

        Throw thrown ->
            handler thrown

        Call op arguments continue ->
            Call op arguments (continue >> catch handler)

        Start op arguments continue ->
            Start op arguments (continue >> catch handler)

        Sleep rest ->
            Sleep (catch handler rest)

        Spawn child continue ->
            Spawn child (continue >> catch handler)

        Await id continue ->
            Await id (continue >> catch handler)

        Settle id outcome rest ->
            Settle id outcome (catch handler rest)

        Deferred continue ->
            Deferred (continue >> catch handler)

        Modify change ->
            Modify (change >> Tuple.mapSecond (catch handler))

        Exit outcome ->
            Exit outcome


attempt : Io w a -> Io w (Result Thrown a)
attempt io =
    map Ok io |> catch (Err >> Done)


{-| `try { io } finally { cleanup }`: the cleanup runs on both paths, then the outcome stands.
-}
finally : Io w () -> Io w a -> Io w a
finally cleanup io =
    attempt io
        |> andThen
            (\outcome ->
                cleanup
                    |> andThen
                        (\() ->
                            case outcome of
                                Ok value ->
                                    Done value

                                Err thrown ->
                                    Throw thrown
                        )
            )


{-| Runs the actions one after the other, collecting their results.
-}
sequence : List (Io w a) -> Io w (List a)
sequence actions =
    let
        go done remaining =
            case remaining of
                [] ->
                    Done (List.reverse done)

                action :: rest ->
                    action |> andThen (\value -> go (value :: done) rest)
    in
    go [] actions


forEach : (a -> Io w ()) -> List a -> Io w ()
forEach each items =
    case items of
        [] ->
            Done ()

        item :: rest ->
            each item |> andThen (\() -> forEach each rest)


foldl : (a -> b -> Io w b) -> b -> List a -> Io w b
foldl step value items =
    case items of
        [] ->
            Done value

        item :: rest ->
            step item value |> andThen (\next -> foldl step next rest)


modify : (w -> ( w, a )) -> Io w a
modify change =
    Modify (change >> Tuple.mapSecond Done)


get : (w -> a) -> Io w a
get read =
    Modify (\world -> ( world, Done (read world) ))


{-| Starts `io` as its own fiber, at once, and gives its promise.
-}
spawn : (a -> E.Value) -> D.Decoder a -> Io w a -> Io w (Promise a)
spawn encode decoder io =
    Spawn (map encode io) (\id -> Done (Promise id decoder))


{-| `await promise`: its value, or what it rejected with.
-}
await : Promise a -> Io w a
await (Promise id decoder) =
    Await id
        (\outcome ->
            case outcome of
                Ok value ->
                    case D.decodeValue decoder value of
                        Ok decoded ->
                            Done decoded

                        Err _ ->
                            Exit (Err "a promise settled with a value of another type")

                Err thrown ->
                    Throw thrown
        )


{-| Settles a deferred promise; a settled promise stays as it is.
-}
settle : Promise a -> Result Thrown E.Value -> Io w ()
settle (Promise id _) outcome =
    Settle id outcome (Done ())


{-| workbox-core's `Deferred`: a promise and the means to settle it.
-}
deferred : D.Decoder a -> Io w (Promise a)
deferred decoder =
    Deferred (\id -> Done (Promise id decoder))


sleep : Io w ()
sleep =
    Sleep (Done ())


reply : String -> D.Decoder a -> Result JsError D.Value -> Io w a
reply op decoder outcome =
    case outcome of
        Ok value ->
            case D.decodeValue decoder value of
                Ok decoded ->
                    Done decoded

                Err _ ->
                    Exit (Err ("unexpected host answer to " ++ op))

        Err error ->
            Throw (HostError error)


{-| A host primitive that answers at once, or throws.
-}
call : String -> List ( String, E.Value ) -> D.Decoder a -> Io w a
call op arguments decoder =
    Call op arguments (reply op decoder)


{-| A host primitive that answers with a promise: its value, or what it rejected with.
-}
start : String -> List ( String, E.Value ) -> D.Decoder a -> Io w a
start op arguments decoder =
    Start op arguments (reply op decoder)


exit : Result String E.Value -> Io w a
exit =
    Exit


{-| `value instanceof Error`, the guard Workbox applies before it keeps or reports a caught value.
-}
isError : Thrown -> Bool
isError thrown =
    case thrown of
        WorkboxError _ _ ->
            True

        HostError error ->
            error.isError


{-| An Error's `name`; a thrown non-Error prints as its string.
-}
nameOf : Thrown -> String
nameOf thrown =
    case thrown of
        WorkboxError name _ ->
            name

        HostError error ->
            if error.isError then
                error.name

            else
                error.text


{-| The production `messageGenerator`: `code :: JSON.stringify([details])`.
-}
workboxMessage : String -> Maybe E.Value -> String
workboxMessage name details =
    name ++ " :: " ++ E.encode 0 (E.list identity [ Maybe.withDefault E.null details ])


{-| What `JSON.stringify` prints for an error nested in a WorkboxError's details: a WorkboxError's
own enumerable properties (`name`, `details`), and nothing for a native Error.
-}
asDetail : Thrown -> E.Value
asDetail thrown =
    case thrown of
        WorkboxError name details ->
            E.object (( "name", E.string name ) :: (details |> Maybe.map (\value -> [ ( "details", value ) ]) |> Maybe.withDefault []))

        HostError _ ->
            E.object []
