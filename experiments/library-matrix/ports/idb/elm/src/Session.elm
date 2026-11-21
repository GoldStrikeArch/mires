module Session exposing (run)

{-| Plays a script through the port's idb as `libraries/idb/reference.mjs` drives the npm package,
then dumps the databases it left and deletes every database it touched, also after a failure.
The state the reference keeps in variables (connections, opened, attempted, live) is a `Session`
value passed along the steps; the log is the runtime's, appended with `Io.log`.
-}

import Dict exposing (Dict)
import Idb
import Io exposing (Io)
import Json.Decode as D
import Json.Encode as E
import Script exposing (Action(..), Argument(..), Blocked(..), Blocking(..), Op(..), Query(..), Upgrade(..), WalkKind(..))


type alias Session =
    { prefix : String
    , connections : Dict String Idb.Database
    , opened : List Idb.Database
    , attempted : List String
    , live : List String
    }


{-| Why a step did not succeed: a host failure not logged yet, a failure the step where it
happened has logged, or an input outside the contract, which rejects the call after cleanup and
is never logged.
-}
type Failure
    = Threw String
    | Failed
    | OutOfContract String


type alias Outcome a =
    Io (Result Failure a)


run : String -> List Script.Step -> Io ()
run uuid script =
    let
        session =
            { prefix = "mires-matrix-idb-" ++ uuid ++ "-", connections = Dict.empty, opened = [], attempted = [], live = [] }
    in
    steps session Nothing script
        |> Io.andThen
            (\( played, result ) ->
                case result of
                    Err (OutOfContract _) ->
                        cleanup played |> Io.andThen (\cleaned -> Io.Fail (failureName cleaned "TypeError"))

                    _ ->
                        -- A logged failure ends the script there; then the dump and the cleanup.
                        sequenceOk (dump played) played.live
                            |> Io.andThen
                                (\dumps ->
                                    cleanup played
                                        |> Io.andThen
                                            (\cleaned ->
                                                case ( dumps, cleaned ) of
                                                    ( Ok databases, Ok () ) ->
                                                        Io.Finish (\log -> E.object [ ( "log", E.list identity log ), ( "databases", E.list identity databases ) ])

                                                    ( Err error, Ok () ) ->
                                                        Io.Fail error

                                                    ( _, Err error ) ->
                                                        Io.Fail error
                                            )
                                )
            )


{-| As the reference's `finally`: a cleanup failure replaces the result.
-}
failureName : Result String () -> String -> String
failureName cleaned otherwise =
    case cleaned of
        Err error ->
            error

        Ok () ->
            otherwise



-- OUTCOMES


ok : a -> Outcome a
ok found =
    Io.succeed (Ok found)


outside : String -> Outcome a
outside message =
    Io.succeed (Err (OutOfContract message))


host : Io (Result String a) -> Outcome a
host =
    Io.map (Result.mapError Threw)


andThenOk : (a -> Outcome b) -> Outcome a -> Outcome b
andThenOk next =
    Io.andThen
        (\result ->
            case result of
                Ok found ->
                    next found

                Err failure ->
                    Io.succeed (Err failure)
        )


entry : List E.Value -> E.Value
entry =
    E.list identity


{-| Logs a failure where it happens; the `Failed` that propagates says it is logged.
-}
failed : String -> Failure -> Outcome a
failed label failure =
    case failure of
        Threw name ->
            Io.log (entry [ E.string label, E.string "error", E.string name ]) |> Io.map (\_ -> Err Failed)

        _ ->
            Io.succeed (Err failure)


{-| One call: logs `[label, "ok", ...show value]` or `[label, "error", name]`.
-}
attempt : String -> (a -> List E.Value) -> Outcome a -> Outcome a
attempt label show action =
    action
        |> Io.andThen
            (\result ->
                case result of
                    Ok found ->
                        Io.log (entry (E.string label :: E.string "ok" :: show found)) |> Io.map (\_ -> Ok found)

                    Err failure ->
                        failed label failure
            )


{-| A host result as the log shows it: nothing when it is undefined (a miss), else the value.
-}
shown : Maybe D.Value -> List E.Value
shown =
    Maybe.map List.singleton >> Maybe.withDefault []


none : a -> List E.Value
none _ =
    []


connection : Session -> String -> Outcome Idb.Database
connection session conn =
    case Dict.get conn session.connections of
        Just db ->
            ok db

        Nothing ->
            outside ("no connection named " ++ conn)


inside : Maybe Idb.Transaction -> (Idb.Transaction -> Outcome a) -> Outcome a
inside tx next =
    case tx of
        Just active ->
            next active

        Nothing ->
            outside "a transaction step outside a transaction"


remember : String -> List String -> List String
remember name names =
    if List.member name names then
        names

    else
        names ++ [ name ]



-- STEPS


steps : Session -> Maybe Idb.Transaction -> List Script.Step -> Io ( Session, Result Failure () )
steps session tx list =
    case list of
        [] ->
            Io.succeed ( session, Ok () )

        current :: rest ->
            perform session tx current
                |> Io.andThen
                    (\( next, result ) ->
                        case result of
                            Ok () ->
                                steps next tx rest

                            Err failure ->
                                Io.succeed ( next, Err failure )
                    )


{-| A step with `catch` stops a failure of its own, or of a step inside it, here.
-}
perform : Session -> Maybe Idb.Transaction -> Script.Step -> Io ( Session, Result Failure () )
perform session tx step =
    execute session tx step.op
        |> Io.map
            (\( next, result ) ->
                case result of
                    Err Failed ->
                        if step.catch then
                            ( next, Ok () )

                        else
                            ( next, Err Failed )

                    _ ->
                        ( next, result )
            )


{-| A step that leaves the session as it is.
-}
same : Session -> Outcome a -> Io ( Session, Result Failure () )
same session =
    Io.map (\result -> ( session, Result.map (\_ -> ()) result ))


execute : Session -> Maybe Idb.Transaction -> Op -> Io ( Session, Result Failure () )
execute session tx op =
    case op of
        OpenDb step ->
            open session step

        Close conn ->
            attempt "close" none (connection session conn |> andThenOk (\db -> Idb.close db |> Io.map Ok)) |> same session

        DeleteDb step ->
            remove session step.db step.blocked

        Tx step ->
            transaction session step

        Call step ->
            attempt step.label shown
                (connection session step.conn
                    |> andThenOk (\db -> arguments step.args |> andThenOk (\args -> host (Idb.shortcut step.shortcut db step.store step.index args)))
                )
                |> same session

        Batch step ->
            inside tx (\t -> batch t step) |> same session

        Walk walkStep ->
            case walkStep.kind of
                Iterate ->
                    iterate tx walkStep |> same session

                _ ->
                    walk tx walkStep |> same session

        TxAbort ->
            attempt "abort" none (inside tx (Idb.abort >> host)) |> same session

        TxDone ->
            attempt "done" none (inside tx (\t -> awaitDone t)) |> same session

        TxStore ->
            attempt "store" shown (inside tx storeName) |> same session

        Wrap step ->
            attempt "wrap" shown
                (source tx step.target
                    |> andThenOk (\from -> arguments step.args |> andThenOk (\args -> host (Idb.wrap (Idb.unwrap from) step.method args)))
                    |> andThenOk (Idb.await >> host)
                )
                |> same session

        Request step ->
            attempt (Idb.methodName step.method) shown (request tx step |> andThenOk (Idb.await >> host)) |> same session


awaitDone : Idb.Transaction -> Outcome ()
awaitDone tx =
    case Idb.done tx of
        Just finished ->
            host (Idb.await finished)

        Nothing ->
            outside "done of a versionchange transaction"


{-| `tx.store?.name`.
-}
storeName : Idb.Transaction -> Outcome (Maybe D.Value)
storeName tx =
    host (Idb.store tx)
        |> andThenOk
            (\only ->
                case only of
                    Just found ->
                        Idb.name (Idb.storeSource found) |> Io.map (Just >> Ok)

                    Nothing ->
                        ok Nothing
            )


{-| The source of a request: `store` (or `tx.store`), then its `index`. Its errors are the step's.
-}
source : Maybe Idb.Transaction -> Script.Target -> Outcome Idb.Source
source tx target =
    inside tx
        (\t ->
            (case target.store of
                Just name ->
                    host (Idb.objectStore t name)

                Nothing ->
                    host (Idb.store t)
                        |> andThenOk
                            (\only ->
                                case only of
                                    Just found ->
                                        ok found

                                    Nothing ->
                                        outside "store: null in a transaction with several stores"
                            )
            )
                |> andThenOk
                    (\found ->
                        case target.index of
                            Just name ->
                                host (Idb.index found name) |> Io.map (Result.map Idb.indexSource)

                            Nothing ->
                                ok (Idb.storeSource found)
                    )
        )


{-| The arguments as values; a key range is built here, before the call.
-}
arguments : List Argument -> Outcome (List D.Value)
arguments args =
    let
        value argument =
            case argument of
                Value found ->
                    ok found

                Range range ->
                    host (keyRange range)
    in
    Io.sequence value args
        |> Io.map (List.foldr (Result.map2 (::)) (Ok []))


keyRange : Query -> Io (Result String D.Value)
keyRange range =
    case range of
        Only only ->
            Idb.keyRange "only" [ only ]

        Lower lower lowerOpen ->
            Idb.keyRange "lowerBound" [ lower, lowerOpen ]

        Upper upper upperOpen ->
            Idb.keyRange "upperBound" [ upper, upperOpen ]

        Bound lower upper lowerOpen upperOpen ->
            Idb.keyRange "bound" [ lower, upper, lowerOpen, upperOpen ]


{-| `source[method](...args)`: the source, then the arguments, then the request, as JS evaluates
`source(tx, step)[op](...args(op, step.args))`.
-}
request : Maybe Idb.Transaction -> Script.RequestStep -> Outcome (Idb.Promise (Maybe D.Value))
request tx step =
    source tx step.target
        |> andThenOk (\from -> arguments step.args |> andThenOk (\args -> host (Idb.request step.method from args)))



-- OPEN AND DELETE


closingConnection : Session -> Maybe Blocked -> Outcome (Maybe Idb.Database)
closingConnection session blockedBy =
    case blockedBy of
        Just (BlockedClose conn) ->
            connection session conn |> Io.map (Result.map Just)

        _ ->
            ok Nothing


closeIfAny : Maybe Idb.Database -> Io ()
closeIfAny =
    Maybe.map Idb.close >> Maybe.withDefault (Io.succeed ())


open : Session -> Script.OpenStep -> Io ( Session, Result Failure () )
open session step =
    let
        tried =
            { session | attempted = remember step.db session.attempted }

        callbacks closing =
            { upgrade =
                Maybe.map
                    (\ops db oldVersion newVersion upgradeTx ->
                        Io.log (entry [ E.string "upgrade", oldVersion, newVersion ])
                            |> Io.andThen (\_ -> Io.sequence (upgrade db upgradeTx) ops)
                            |> Io.map (\_ -> ())
                    )
                    step.upgrade
            , blocked =
                Maybe.map
                    (\_ currentVersion blockedVersion ->
                        Io.log (entry [ E.string "blocked", currentVersion, blockedVersion ]) |> Io.andThen (\_ -> closeIfAny closing)
                    )
                    step.blocked
            , blocking =
                Maybe.map
                    (\kind currentVersion blockedVersion db ->
                        Io.log (entry [ E.string "blocking", E.string step.conn, currentVersion, blockedVersion ])
                            |> Io.andThen
                                (\_ ->
                                    if kind == BlockingClose then
                                        Idb.close db

                                    else
                                        Io.succeed ()
                                )
                    )
                    step.blocking
            }

        describe db =
            Idb.version db
                |> Io.andThen (\version -> Idb.objectStoreNames db |> Io.map (\names -> Ok ( db, entry [ version, E.list E.string names ] )))
    in
    closingConnection tried step.blocked
        |> andThenOk
            (\closing ->
                attempt "openDB"
                    (\( _, description ) -> [ description ])
                    (host (Idb.openDB (tried.prefix ++ step.db) step.version (callbacks closing)) |> andThenOk describe)
            )
        |> Io.map
            (\result ->
                case result of
                    Ok ( db, _ ) ->
                        ( { tried
                            | connections = Dict.insert step.conn db tried.connections
                            , opened = tried.opened ++ [ db ]
                            , live = remember step.db tried.live
                          }
                        , Ok ()
                        )

                    Err failure ->
                        ( tried, Err failure )
            )


remove : Session -> String -> Maybe Blocked -> Io ( Session, Result Failure () )
remove session db blockedBy =
    let
        tried =
            { session | attempted = remember db session.attempted }

        callback closing =
            Maybe.map
                (\_ currentVersion -> Io.log (entry [ E.string "blocked", currentVersion ]) |> Io.andThen (\_ -> closeIfAny closing))
                blockedBy
    in
    closingConnection tried blockedBy
        |> andThenOk (\closing -> attempt "deleteDB" none (host (Idb.deleteDB (tried.prefix ++ db) (callback closing))))
        |> Io.map
            (\result ->
                case result of
                    Ok () ->
                        ( { tried | live = List.filter ((/=) db) tried.live }, Ok () )

                    Err failure ->
                        ( tried, Err failure )
            )


{-| Runs inside the upgrade callback, one host call at a time; a failure is logged and the callback
goes on.
-}
upgrade : Idb.Database -> Idb.Transaction -> Upgrade -> Io ()
upgrade db tx op =
    let
        logged label result =
            case result of
                Ok shownValues ->
                    Io.log (entry (E.string label :: E.string "ok" :: shownValues))

                Err name ->
                    Io.log (entry [ E.string label, E.string "error", E.string name ])

        named label created =
            created
                |> Io.andThen
                    (\result ->
                        case result of
                            Ok source_ ->
                                Idb.name source_ |> Io.andThen (\name -> logged label (Ok [ name ]))

                            Err name ->
                                logged label (Err name)
                    )

        unit label done =
            done |> Io.andThen (Result.map (\_ -> []) >> logged label)

        inStore name next =
            Idb.objectStore tx name
                |> Io.andThen
                    (\result ->
                        case result of
                            Ok found ->
                                next found

                            Err error ->
                                Io.succeed (Err error)
                    )
    in
    case op of
        CreateStore name options ->
            named "createStore" (Idb.createObjectStore db name options |> Io.map (Result.map Idb.storeSource))

        DeleteStore name ->
            unit "deleteStore" (Idb.deleteObjectStore db name)

        CreateIndex index ->
            named "createIndex" (inStore index.store (\found -> Idb.createIndex found index.name index.keyPath index.options) |> Io.map (Result.map Idb.indexSource))

        DeleteIndex storeName_ indexName ->
            unit "deleteIndex" (inStore storeName_ (\found -> Idb.deleteIndex found indexName))

        UpgradeAbort ->
            unit "abort" (Idb.abort tx)

        Write method storeName_ args ->
            -- Issued, not awaited: only a synchronous failure is logged. A failed write aborts
            -- the upgrade, which rejects the open.
            inStore storeName_ (\found -> Idb.request method (Idb.storeSource found) args)
                |> Io.andThen
                    (\result ->
                        case result of
                            Ok _ ->
                                Io.succeed ()

                            Err name ->
                                logged (Idb.methodName method) (Err name)
                    )



-- TRANSACTIONS


transaction : Session -> { conn : String, stores : D.Value, mode : D.Value, steps : List Script.Step } -> Io ( Session, Result Failure () )
transaction session step =
    connection session step.conn
        |> andThenOk (\db -> host (Idb.transaction db step.stores (Just step.mode)))
        |> Io.andThen
            (\result ->
                case result of
                    Ok tx ->
                        steps session (Just tx) step.steps

                    Err failure ->
                        failed "tx" failure |> same session
            )


{-| Issues every request before awaiting any, then logs each outcome in issue order. A batch never
fails.
-}
batch : Idb.Transaction -> { requests : List Script.RequestStep, abort : Bool, done : Bool } -> Outcome ()
batch tx step =
    let
        aborted =
            if step.abort then
                attempt "abort" none (host (Idb.abort tx)) |> Io.map (\_ -> ())

            else
                Io.succeed ()

        settle issued =
            case issued of
                Ok promise ->
                    host (Idb.await promise) |> Io.map (Result.map shown)

                Err failure ->
                    Io.succeed (Err failure)

        doneOutcome =
            if step.done then
                awaitDone tx |> Io.map (\result -> [ ( "done", Result.map none result ) ])

            else
                Io.succeed []

        logOutcome ( label, result ) =
            case result of
                Ok values ->
                    Io.log (entry (E.string label :: E.string "ok" :: values))

                Err (Threw name) ->
                    Io.log (entry [ E.string label, E.string "error", E.string name ])

                Err _ ->
                    Io.succeed ()

        outsideOf ( _, result ) =
            case result of
                Err (OutOfContract message) ->
                    Just message

                _ ->
                    Nothing
    in
    Io.sequence (request (Just tx)) step.requests
        |> Io.andThen
            (\issued ->
                aborted
                    |> Io.andThen (\_ -> Io.sequence settle issued)
                    |> Io.andThen
                        (\outcomes ->
                            doneOutcome |> Io.map (\last -> List.map2 Tuple.pair (List.map (.method >> Idb.methodName) step.requests) outcomes ++ last)
                        )
                    |> Io.andThen
                        (\outcomes ->
                            case List.filterMap outsideOf outcomes |> List.head of
                                Just message ->
                                    outside message

                                Nothing ->
                                    Io.sequence logOutcome outcomes |> Io.map (\_ -> Ok ())
                        )
            )



-- CURSORS


actions : Script.WalkStep -> Int -> List Action
actions step position =
    List.drop position step.positions |> List.head |> Maybe.withDefault step.rest


logCursor : Bool -> Idb.Cursor -> Io ()
logCursor withValue cursor =
    Idb.key cursor
        |> Io.andThen
            (\key ->
                Idb.primaryKey cursor
                    |> Io.andThen
                        (\primaryKey ->
                            if withValue then
                                Idb.value cursor |> Io.andThen (\value -> Io.log (entry [ E.string "cursor", key, primaryKey, value ]))

                            else
                                Io.log (entry [ E.string "cursor", key, primaryKey ])
                        )
            )


{-| `cursor.update(value)` and `cursor.delete()`, awaited and logged.
-}
write : Idb.Cursor -> Action -> Outcome ()
write cursor action =
    let
        logged label issued =
            attempt label shown (host issued |> andThenOk (Idb.await >> host)) |> Io.map (Result.map (\_ -> ()))
    in
    case action of
        Update newValue ->
            logged "update" (Idb.update cursor newValue)

        _ ->
            logged "delete" (Idb.delete cursor)


moveName : Action -> String
moveName action =
    case action of
        Advance _ ->
            "advance"

        ContinuePrimaryKey _ ->
            "continuePrimaryKey"

        _ ->
            "continue"


walkLabel : WalkKind -> String
walkLabel kind =
    case kind of
        KeyCursorWalk ->
            "openKeyCursor"

        CursorWalk ->
            "openCursor"

        Iterate ->
            "iterate"


{-| A walk with the promise-returning moves; a position without a move ends it.
-}
walk : Maybe Idb.Transaction -> Script.WalkStep -> Outcome ()
walk tx step =
    let
        label =
            walkLabel step.kind

        opening from args =
            if step.kind == KeyCursorWalk then
                Idb.openKeyCursor from args

            else
                Idb.openCursor from args

        -- The position's actions, one at a time: the last move, or Nothing after a break or
        -- without one, which both end the walk.
        act cursor pending moved =
            case pending of
                [] ->
                    ok moved

                Break :: _ ->
                    ok Nothing

                ((Update _) as action) :: rest ->
                    write cursor action |> andThenOk (\_ -> act cursor rest moved)

                DeleteRecord :: rest ->
                    write cursor DeleteRecord |> andThenOk (\_ -> act cursor rest moved)

                action :: rest ->
                    host (makeMove cursor action)
                        |> Io.andThen
                            (\result ->
                                case result of
                                    Ok promise ->
                                        act cursor rest (Just ( moveName action, promise ))

                                    Err failure ->
                                        failed (moveName action) failure
                            )

        visit found position =
            case found of
                Nothing ->
                    Io.log (entry [ E.string "end" ]) |> Io.map (\_ -> Ok ())

                Just cursor ->
                    logCursor (step.kind /= KeyCursorWalk) cursor
                        |> Io.andThen (\_ -> act cursor (actions step position) Nothing)
                        |> andThenOk
                            (\moved ->
                                case moved of
                                    Nothing ->
                                        ok ()

                                    Just ( moveLabel, promise ) ->
                                        Idb.await promise
                                            |> Io.andThen
                                                (\next ->
                                                    case next of
                                                        Ok cursorAfter ->
                                                            visit cursorAfter (position + 1)

                                                        Err name ->
                                                            failed moveLabel (Threw name)
                                                )
                            )
    in
    source tx step.target
        |> andThenOk (\from -> arguments step.args |> andThenOk (\args -> host (opening from args)))
        |> andThenOk (Idb.await >> host)
        |> Io.andThen
            (\result ->
                case result of
                    Ok found ->
                        visit found 0

                    Err failure ->
                        failed label failure
            )


makeMove : Idb.Cursor -> Action -> Io (Result String (Idb.Promise (Maybe Idb.Cursor)))
makeMove cursor action =
    case action of
        Advance args ->
            Idb.advance cursor args

        ContinuePrimaryKey args ->
            Idb.continuePrimaryKey cursor args

        Continue args ->
            Idb.continue cursor args

        _ ->
            Idb.continue cursor []


{-| The async iterator: a move only records the next step, and a position without one continues.
-}
iterate : Maybe Idb.Transaction -> Script.WalkStep -> Outcome ()
iterate tx step =
    let
        -- The position's actions, one at a time: the iterator after them, or Nothing after a break.
        body iterator cursor pending =
            case pending of
                [] ->
                    ok (Just iterator)

                Break :: _ ->
                    ok Nothing

                ((Update _) as action) :: rest ->
                    write cursor action |> andThenOk (\_ -> body iterator cursor rest)

                DeleteRecord :: rest ->
                    write cursor DeleteRecord |> andThenOk (\_ -> body iterator cursor rest)

                action :: rest ->
                    host (intercepted iterator action)
                        |> Io.andThen
                            (\result ->
                                case result of
                                    Ok moved ->
                                        body moved cursor rest

                                    Err failure ->
                                        failed (moveName action) failure
                            )

        loop iterator position =
            host (Idb.next iterator)
                |> Io.andThen
                    (\result ->
                        case result of
                            Err failure ->
                                failed "iterate" failure

                            Ok Nothing ->
                                Io.log (entry [ E.string "end" ]) |> Io.map (\_ -> Ok ())

                            Ok (Just at) ->
                                case Idb.current at of
                                    Nothing ->
                                        ok ()

                                    Just cursor ->
                                        logCursor True cursor
                                            |> Io.andThen (\_ -> body at cursor (actions step position))
                                            |> andThenOk
                                                (\after ->
                                                    case after of
                                                        Just iterator_ ->
                                                            loop iterator_ (position + 1)

                                                        Nothing ->
                                                            ok ()
                                                )
                    )
    in
    source tx step.target
        |> andThenOk (\from -> arguments step.args |> Io.map (Result.map (Idb.iterate from)))
        |> Io.andThen
            (\result ->
                case result of
                    Ok iterator ->
                        loop iterator 0

                    Err failure ->
                        failed "iterate" failure
            )


intercepted : Idb.Iterator -> Action -> Io (Result String Idb.Iterator)
intercepted iterator action =
    case action of
        Advance args ->
            Idb.iteratedAdvance iterator args

        ContinuePrimaryKey args ->
            Idb.iteratedContinuePrimaryKey iterator args

        Continue args ->
            Idb.iteratedContinue iterator args

        _ ->
            Idb.iteratedContinue iterator []



-- DUMP AND CLEANUP


{-| `[name, version, stores]`, each store `[name, keyPath, autoIncrement, indexes, records]`, read
in one readonly transaction; records are getAllKeys() zipped with getAll().
-}
dump : Session -> String -> Io (Result String E.Value)
dump session name =
    Idb.openDB (session.prefix ++ name) Nothing Idb.noCallbacks
        |> resultThen
            (\db ->
                Idb.objectStoreNames db
                    |> Io.andThen
                        (\names ->
                            (if List.isEmpty names then
                                Io.succeed (Ok [])

                             else
                                dumpStores db names
                            )
                                |> Io.andThen
                                    (\stores ->
                                        Idb.version db
                                            |> Io.andThen
                                                (\version ->
                                                    Idb.close db
                                                        |> Io.map (\_ -> Result.map (\found -> entry [ E.string name, version, E.list identity found ]) stores)
                                                )
                                    )
                        )
            )


dumpStores : Idb.Database -> List String -> Io (Result String (List E.Value))
dumpStores db names =
    Idb.transaction db (E.list E.string names) Nothing
        |> resultThen
            (\tx ->
                sequenceOk (dumpStore tx) names
                    |> resultThen
                        (\stores ->
                            case Idb.done tx of
                                Just finished ->
                                    Idb.await finished |> Io.map (Result.map (\_ -> stores))

                                Nothing ->
                                    Io.succeed (Ok stores)
                        )
            )


dumpStore : Idb.Transaction -> String -> Io (Result String E.Value)
dumpStore tx name =
    let
        describeIndex found indexName =
            Idb.index found indexName
                |> resultThen
                    (\index ->
                        Idb.keyPath (Idb.indexSource index)
                            |> Io.andThen
                                (\path ->
                                    Idb.unique index
                                        |> Io.andThen (\isUnique -> Idb.multiEntry index |> Io.map (\isMulti -> Ok (entry [ E.string indexName, path, E.bool isUnique, E.bool isMulti ])))
                                )
                    )

        all method found =
            Idb.request method (Idb.storeSource found) []
                |> resultThen Idb.await
                |> Io.map (Result.map (Maybe.andThen (D.decodeValue (D.list D.value) >> Result.toMaybe) >> Maybe.withDefault []))
    in
    Idb.objectStore tx name
        |> resultThen
            (\found ->
                Idb.keyPath (Idb.storeSource found)
                    |> Io.andThen
                        (\path ->
                            Idb.autoIncrement found
                                |> Io.andThen
                                    (\isAuto ->
                                        Idb.indexNames found
                                            |> Io.andThen (sequenceOk (describeIndex found))
                                            |> resultThen
                                                (\indexes ->
                                                    all Idb.GetAllKeys found
                                                        |> resultThen
                                                            (\keys ->
                                                                all Idb.GetAll found
                                                                    |> Io.map
                                                                        (Result.map
                                                                            (\values ->
                                                                                entry
                                                                                    [ E.string name
                                                                                    , path
                                                                                    , E.bool isAuto
                                                                                    , E.list identity indexes
                                                                                    , E.list identity (List.map2 (\k v -> entry [ k, v ]) keys values)
                                                                                    ]
                                                                            )
                                                                        )
                                                            )
                                                )
                                    )
                        )
            )


{-| Closes every connection the script opened, then deletes every database it named.
-}
cleanup : Session -> Io (Result String ())
cleanup session =
    Io.sequence Idb.close session.opened
        |> Io.andThen (\_ -> sequenceOk (\name -> Idb.deleteDB (session.prefix ++ name) Nothing) session.attempted)
        |> Io.map (Result.map (\_ -> ()))


resultThen : (a -> Io (Result String b)) -> Io (Result String a) -> Io (Result String b)
resultThen next =
    Io.andThen
        (\result ->
            case result of
                Ok found ->
                    next found

                Err error ->
                    Io.succeed (Err error)
        )


{-| `each` on the items one after another, up to the first failure.
-}
sequenceOk : (a -> Io (Result String b)) -> List a -> Io (Result String (List b))
sequenceOk each =
    let
        go done items =
            case items of
                [] ->
                    Io.succeed (Ok (List.reverse done))

                item :: rest ->
                    each item |> resultThen (\found -> go (found :: done) rest)
    in
    go []
