module Script exposing
    ( Step, Op(..), OpenStep, Blocking(..), Blocked(..), Upgrade(..)
    , Target, RequestStep, Argument(..), Query(..), WalkStep, WalkKind(..), Action(..)
    , decoder
    )

{-| The corpus input (CONTRACT.md, "Input") as Elm types. Names become Elm strings; keys, values,
versions, counts, directions, modes and options stay host values, sent to IndexedDB unchanged. A
field that may be absent is a `Maybe`, so an absent argument is left out of the host call rather
than sent as null. An input outside the contract does not decode, and the call rejects before
any database is opened.
-}

import Idb
import Json.Decode as D
import Json.Encode as E


type alias Step =
    { op : Op
    , catch : Bool
    }


type Op
    = OpenDb OpenStep
    | Close String
    | DeleteDb { db : String, blocked : Maybe Blocked }
    | Tx { conn : String, stores : D.Value, mode : D.Value, steps : List Step }
    | Call { conn : String, label : String, shortcut : Idb.Shortcut, store : String, index : Maybe String, args : List Argument }
    | Request RequestStep
    | Batch { requests : List RequestStep, abort : Bool, done : Bool }
    | Walk WalkStep
    | TxAbort
    | TxDone
    | TxStore
    | Wrap RequestStep


type alias OpenStep =
    { db : String
    , conn : String
    , version : Maybe D.Value
    , upgrade : Maybe (List Upgrade)
    , blocking : Maybe Blocking
    , blocked : Maybe Blocked
    }


type Blocking
    = BlockingLog
    | BlockingClose


type Blocked
    = BlockedLog
    | BlockedClose String


type Upgrade
    = CreateStore String (Maybe D.Value)
    | DeleteStore String
    | CreateIndex { store : String, name : String, keyPath : D.Value, options : Maybe D.Value }
    | DeleteIndex String String
    | Write Idb.Method String (List D.Value)
    | UpgradeAbort


{-| `store: Nothing` is `tx.store`.
-}
type alias Target =
    { store : Maybe String
    , index : Maybe String
    }


type alias RequestStep =
    { method : Idb.Method
    , target : Target
    , args : List Argument
    }


{-| A call's argument; a query method's first argument may be a key range written as an object.
-}
type Argument
    = Value D.Value
    | Range Query


{-| A key range; an absent open flag is false, as IndexedDB reads an undefined one.
-}
type Query
    = Only D.Value
    | Lower D.Value D.Value
    | Upper D.Value D.Value
    | Bound D.Value D.Value D.Value D.Value


type alias WalkStep =
    { kind : WalkKind
    , target : Target
    , args : List Argument
    , positions : List (List Action)
    , rest : List Action
    }


type WalkKind
    = CursorWalk
    | KeyCursorWalk
    | Iterate


{-| A cursor action. A move keeps the arguments it was given: `["continue"]` passes none.
-}
type Action
    = Continue (List D.Value)
    | Advance (List D.Value)
    | ContinuePrimaryKey (List D.Value)
    | Update D.Value
    | DeleteRecord
    | Break


decoder : D.Decoder (List Step)
decoder =
    D.field "steps" steps


steps : D.Decoder (List Step)
steps =
    D.list (D.lazy (\_ -> step))


step : D.Decoder Step
step =
    D.map2 Step
        (D.field "op" D.string |> D.andThen op)
        (optional "catch" D.bool |> D.map (Maybe.withDefault False))


{-| An optional field: absent is `Nothing`, and a present one must decode.
-}
optional : String -> D.Decoder a -> D.Decoder (Maybe a)
optional name inner =
    D.maybe (D.field name D.value)
        |> D.andThen
            (\present ->
                case present of
                    Just _ ->
                        D.field name inner |> D.map Just

                    Nothing ->
                        D.succeed Nothing
            )


op : String -> D.Decoder Op
op name =
    case name of
        "openDB" ->
            D.map OpenDb openStep

        "close" ->
            D.map Close (D.field "conn" D.string)

        "deleteDB" ->
            D.map2 (\db blockedBy -> DeleteDb { db = db, blocked = blockedBy })
                (D.field "db" D.string)
                (optional "blocked" blocked)

        "tx" ->
            D.map4 (\conn stores mode inner -> Tx { conn = conn, stores = stores, mode = mode, steps = inner })
                (D.field "conn" D.string)
                (D.field "stores" D.value)
                (D.field "mode" D.value)
                (D.field "steps" (D.lazy (\_ -> steps)))

        "call" ->
            D.field "method" D.string |> D.andThen call

        "batch" ->
            D.map3 (\requests abort done -> Batch { requests = requests, abort = abort, done = done })
                (D.field "requests" (D.list (D.field "op" D.string |> D.andThen request)))
                (D.field "abort" D.bool)
                (D.field "done" D.bool)

        "openCursor" ->
            D.map Walk (walk CursorWalk)

        "openKeyCursor" ->
            D.map Walk (walk KeyCursorWalk)

        "iterate" ->
            D.map Walk (walk Iterate)

        "abort" ->
            D.succeed TxAbort

        "done" ->
            D.succeed TxDone

        "store" ->
            D.succeed TxStore

        "wrap" ->
            D.field "method" D.string |> D.andThen request |> D.map Wrap

        other ->
            D.map Request (request other)


openStep : D.Decoder OpenStep
openStep =
    D.map6 OpenStep
        (D.field "db" D.string)
        (D.field "conn" D.string)
        (optional "version" D.value)
        (optional "upgrade" (D.list upgrade))
        (optional "blocking" (D.string |> D.map (\kind -> if kind == "close" then BlockingClose else BlockingLog)))
        (optional "blocked" blocked)


blocked : D.Decoder Blocked
blocked =
    D.oneOf
        [ D.string |> D.map (\_ -> BlockedLog)
        , D.field "close" D.string |> D.map BlockedClose
        ]


upgrade : D.Decoder Upgrade
upgrade =
    D.field "op" D.string
        |> D.andThen
            (\name ->
                case name of
                    "createStore" ->
                        D.map2 CreateStore (D.field "name" D.string) (optional "options" D.value)

                    "deleteStore" ->
                        D.map DeleteStore (D.field "name" D.string)

                    "createIndex" ->
                        D.map4 (\store indexName path options -> CreateIndex { store = store, name = indexName, keyPath = path, options = options })
                            (D.field "store" D.string)
                            (D.field "name" D.string)
                            (D.field "keyPath" D.value)
                            (optional "options" D.value)

                    "deleteIndex" ->
                        D.map2 DeleteIndex (D.field "store" D.string) (D.field "name" D.string)

                    "put" ->
                        D.map2 (Write Idb.Put) (D.field "store" D.string) (D.field "args" (D.list D.value))

                    "add" ->
                        D.map2 (Write Idb.Add) (D.field "store" D.string) (D.field "args" (D.list D.value))

                    "abort" ->
                        D.succeed UpgradeAbort

                    _ ->
                        D.fail ("not an upgrade step: " ++ name)
            )


call : String -> D.Decoder Op
call label =
    case Idb.shortcutOf label of
        Nothing ->
            D.fail ("not a database shortcut: " ++ label)

        Just shortcut ->
            D.map4 (\conn store indexName args -> { conn = conn, label = label, shortcut = shortcut, store = store, index = indexName, args = args })
                (D.field "conn" D.string)
                (D.field "store" D.string)
                (D.field "index" (D.nullable D.string))
                (D.field "args" (arguments (Idb.takesQuery (Idb.shortcutMethod shortcut))))
                |> D.andThen
                    (\step_ ->
                        if Idb.usesIndex shortcut == (step_.index /= Nothing) then
                            D.succeed (Call step_)

                        else
                            D.fail "an index goes with the FromIndex forms only"
                    )


request : String -> D.Decoder RequestStep
request name =
    case Idb.methodOf name of
        Nothing ->
            D.fail ("not a request method: " ++ name)

        Just method ->
            D.map2 (RequestStep method) target (D.field "args" (arguments (Idb.takesQuery method)))


target : D.Decoder Target
target =
    D.map2 Target
        (D.field "store" (D.nullable D.string))
        (D.field "index" (D.nullable D.string))


walk : WalkKind -> D.Decoder WalkStep
walk kind =
    D.map4 (WalkStep kind)
        target
        (D.field "args" (arguments True))
        (D.field "positions" (D.list (D.list action)))
        (D.field "rest" (D.list action))


action : D.Decoder Action
action =
    D.index 0 D.string
        |> D.andThen
            (\name ->
                D.list D.value
                    |> D.map (List.drop 1)
                    |> D.andThen
                        (\values ->
                            case name of
                                "continue" ->
                                    D.succeed (Continue values)

                                "advance" ->
                                    D.succeed (Advance values)

                                "continuePrimaryKey" ->
                                    D.succeed (ContinuePrimaryKey values)

                                "update" ->
                                    D.index 1 D.value |> D.map Update

                                "delete" ->
                                    D.succeed DeleteRecord

                                "break" ->
                                    D.succeed Break

                                _ ->
                                    D.fail ("not a cursor action: " ++ name)
                        )
            )


{-| A call's arguments, as many as there are. With a query first, an object is a key range.
-}
arguments : Bool -> D.Decoder (List Argument)
arguments queryFirst =
    D.list D.value
        |> D.map
            (\values ->
                case values of
                    first :: rest ->
                        (if queryFirst then
                            query first

                         else
                            Value first
                        )
                            :: List.map Value rest

                    [] ->
                        []
            )


{-| A key is passed as it is (null, a string, a number or an array); an object is a key range.
Presence is tested with `in` (`Json.Decode.field`), so `{ only: null }` is only(null).
-}
query : D.Value -> Argument
query raw =
    let
        has name =
            D.decodeValue (D.field name D.value) raw |> Result.toMaybe

        get name =
            has name |> Maybe.withDefault E.null

        flag name =
            has name |> Maybe.withDefault (E.bool False)
    in
    case D.decodeValue (D.keyValuePairs D.value) raw of
        Err _ ->
            Value raw

        Ok _ ->
            Range
                (case ( has "only", has "lower", has "upper" ) of
                    ( Just only, _, _ ) ->
                        Only only

                    ( Nothing, Just lower, Just upper ) ->
                        Bound lower upper (flag "lowerOpen") (flag "upperOpen")

                    ( Nothing, Just lower, Nothing ) ->
                        Lower lower (flag "lowerOpen")

                    ( Nothing, Nothing, _ ) ->
                        Upper (get "upper") (flag "upperOpen")
                )

