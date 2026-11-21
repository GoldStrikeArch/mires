module Idb exposing
    ( Database, Transaction, ObjectStore, Index, Source, Cursor, Promise
    , await, wrap, unwrap
    , openDB, OpenCallbacks, noCallbacks, deleteDB
    , version, objectStoreNames, createObjectStore, deleteObjectStore, transaction, close
    , done, store, objectStore, abort
    , storeSource, indexSource, index, createIndex, deleteIndex, name, keyPath, autoIncrement, indexNames, unique, multiEntry
    , Method(..), methodName, methodOf, isWrite, takesQuery, request, openCursor, openKeyCursor
    , key, primaryKey, value, advance, continue, continuePrimaryKey, update, delete
    , Shortcut, shortcutOf, shortcutMethod, usesIndex, shortcut
    , Iterator, iterate, next, current, iteratedAdvance, iteratedContinue, iteratedContinuePrimaryKey
    , keyRange
    )

{-| idb 8.0.4 (build/index.js) in Elm: IndexedDB requests become promises, a transaction gets a
`done` promise when it is wrapped, cursors move by returning a promise of the next cursor, the
database has the database-extras shortcuts, and `iterate` walks a store or an index.

idb wraps each IDB object in a Proxy that forwards every property and wraps what comes back. Here
each wrapped object is an opaque host value; its members are host primitives (`Io.call`), and what
they give back is wrapped the same way. A failure is the name of what the host threw or rejected
with. A request is issued when its function runs, and a host exception comes back at once, as
`Err`, before any promise exists.

-}

import Io exposing (Io)
import Json.Decode as D
import Json.Encode as E


type Database
    = Database D.Value


{-| A wrapped transaction. Its `done` promise is the id of the `transaction` command, whose events
settle it. The versionchange transaction an upgrade receives has none: the `open` primitive
reports no events of that transaction, and nothing in the subset awaits it.
-}
type Transaction
    = Transaction { raw : D.Value, done : Maybe Int }


type ObjectStore
    = ObjectStore D.Value


type Index
    = Index D.Value


{-| An object store or one of its indexes: the request methods both have.
-}
type Source
    = StoreSource ObjectStore
    | IndexSource Index


type Cursor
    = Cursor D.Value


{-| A promise: its id, and how its value is read (idb's `wrap(request.result)`).
-}
type Promise a
    = Promise Int (Maybe D.Value -> a)


await : Promise a -> Io (Result String a)
await (Promise id wrapResult) =
    Io.await id |> Io.map (Result.map wrapResult)



-- REQUESTS


{-| The result field of a host message; `Nothing` when it is undefined (the key is absent).
-}
result : D.Value -> Maybe D.Value
result message =
    D.decodeValue (D.maybe (D.field "result" D.value)) message |> Result.withDefault Nothing


nameOf : D.Value -> String
nameOf message =
    D.decodeValue (D.field "name" D.string) message |> Result.withDefault "Error"


{-| idb's `promisifyRequest` listeners: the first `success` or `error` settles the request's
promise, with the request's result or its error, then both are removed.
-}
promisified : Int -> Io.Event -> Io ()
promisified id event =
    let
        settleOnce settled =
            Io.settle id settled |> Io.andThen (\_ -> Io.unlisten id)
    in
    case event.kind of
        "success" ->
            settleOnce (Ok (result event.message))

        "error" ->
            settleOnce (Err (nameOf event.message))

        _ ->
            Io.succeed ()


{-| A request primitive (`request` or `step`): the host call is made, and if it returned, its
request is promisified. A cursor request fires once per move, so each move promisifies it again.
-}
issue : String -> List ( String, E.Value ) -> (Maybe D.Value -> a) -> Io (Result String (Promise a))
issue op fields wrapResult =
    Io.call op fields
        |> Io.andThen
            (\reply ->
                case reply.result of
                    Err threw ->
                        Io.succeed (Err threw)

                    Ok _ ->
                        Io.listen reply.id (promisified reply.id) |> Io.map (\_ -> Ok (Promise reply.id wrapResult))
            )


{-| idb's `wrap(target[method](...args))`: a request on a host object, promisified.
-}
wrap : D.Value -> Method -> List D.Value -> Io (Result String (Promise (Maybe D.Value)))
wrap target method args =
    issue "request" [ ( "target", target ), ( "method", E.string (methodName method) ), ( "args", E.list identity args ) ] identity


cursorResult : Maybe D.Value -> Maybe Cursor
cursorResult raw =
    case raw of
        Just found ->
            case D.decodeValue (D.null ()) found of
                Ok () ->
                    Nothing

                Err _ ->
                    Just (Cursor found)

        Nothing ->
            Nothing


{-| idb's `unwrap`: the host object behind a store or an index.
-}
unwrap : Source -> D.Value
unwrap source =
    case source of
        StoreSource (ObjectStore raw) ->
            raw

        IndexSource (Index raw) ->
            raw


{-| A host method that is not a request (objectStore, index, createObjectStore, abort, ...).
-}
callMethod : D.Value -> String -> List E.Value -> Io (Result String D.Value)
callMethod target method args =
    Io.call "call" [ ( "target", target ), ( "method", E.string method ), ( "args", E.list identity args ) ]
        |> Io.map Io.objectResult


read : D.Value -> String -> Io (Maybe D.Value)
read target property =
    Io.call "read" [ ( "target", target ), ( "property", E.string property ) ]
        |> Io.map (\reply -> reply.result |> Result.toMaybe |> Maybe.andThen identity)


readValue : D.Value -> String -> Io D.Value
readValue target property =
    read target property |> Io.map (Maybe.withDefault E.null)


readNames : D.Value -> String -> Io (List String)
readNames target property =
    readValue target property |> Io.map (D.decodeValue (D.list D.string) >> Result.withDefault [])


readBool : D.Value -> String -> Io Bool
readBool target property =
    readValue target property |> Io.map (D.decodeValue D.bool >> Result.withDefault False)


optionalField : String -> Maybe D.Value -> List ( String, E.Value )
optionalField fieldName maybe =
    case maybe of
        Just present ->
            [ ( fieldName, present ) ]

        Nothing ->
            []


optionalArgument : Maybe D.Value -> List E.Value
optionalArgument =
    Maybe.map List.singleton >> Maybe.withDefault []



-- DATABASE


type alias OpenCallbacks =
    { upgrade : Maybe (Database -> D.Value -> D.Value -> Transaction -> Io ())
    , blocked : Maybe (D.Value -> D.Value -> Io ())
    , blocking : Maybe (D.Value -> D.Value -> Database -> Io ())
    }


noCallbacks : OpenCallbacks
noCallbacks =
    { upgrade = Nothing, blocked = Nothing, blocking = Nothing }


{-| idb's `openDB(name, version, { upgrade, blocked, blocking })`. Without a version it opens the
current version, or 1 for a new database; an invalid one throws. `upgrade(db, oldVersion,
newVersion, transaction)` runs inside `upgradeneeded`, `blocked(currentVersion, blockedVersion)`
on the request's `blocked`. `blocking(currentVersion, blockedVersion, db)` is attached to the
connection once the open succeeds, before the caller resumes; idb attaches it in a `then` on the
open promise, which runs before the caller's continuation too.

The open has one event handler: idb's upgradeneeded and blocked listeners, and the promisified
success and error.

-}
openDB : String -> Maybe D.Value -> OpenCallbacks -> Io (Result String Database)
openDB databaseName requested callbacks =
    Io.call "open" (( "name", E.string databaseName ) :: optionalField "version" requested)
        |> Io.andThen
            (\reply ->
                case reply.result of
                    Err threw ->
                        Io.succeed (Err threw)

                    Ok _ ->
                        Io.listen reply.id (openEvent reply.id callbacks)
                            |> Io.andThen (\_ -> Io.await reply.id)
                            |> Io.andThen
                                (\settled ->
                                    case settled of
                                        Err error ->
                                            Io.succeed (Err error)

                                        Ok opened ->
                                            let
                                                db =
                                                    Database (Maybe.withDefault E.null opened)
                                            in
                                            attachBlocking db callbacks.blocking |> Io.map (\_ -> Ok db)
                                )
            )


openEvent : Int -> OpenCallbacks -> Io.Event -> Io ()
openEvent id callbacks event =
    let
        versions callback =
            callback (Io.field "oldVersion" event.message) (Io.field "newVersion" event.message)
    in
    case ( event.kind, callbacks.upgrade, callbacks.blocked ) of
        ( "upgradeneeded", Just upgrade, _ ) ->
            versions (upgrade (Database (Io.field "db" event.message)))
                (Transaction { raw = Io.field "transaction" event.message, done = Nothing })

        ( "blocked", _, Just blocked ) ->
            versions blocked

        _ ->
            promisified id event


attachBlocking : Database -> Maybe (D.Value -> D.Value -> Database -> Io ()) -> Io ()
attachBlocking (Database db) blocking =
    case blocking of
        Nothing ->
            Io.succeed ()

        Just callback ->
            Io.call "listenVersionChange" [ ( "db", db ) ]
                |> Io.andThen
                    (\reply ->
                        Io.listen reply.id
                            (\event ->
                                if event.kind == "versionchange" then
                                    callback (Io.field "oldVersion" event.message) (Io.field "newVersion" event.message) (Database db)

                                else
                                    Io.succeed ()
                            )
                    )


{-| idb's `deleteDB(name, { blocked })`; `blocked(currentVersion)`.
-}
deleteDB : String -> Maybe (D.Value -> Io ()) -> Io (Result String ())
deleteDB databaseName blocked =
    Io.call "deleteDatabase" [ ( "name", E.string databaseName ) ]
        |> Io.andThen
            (\reply ->
                case reply.result of
                    Err threw ->
                        Io.succeed (Err threw)

                    Ok _ ->
                        Io.listen reply.id
                            (\event ->
                                case ( event.kind, blocked ) of
                                    ( "blocked", Just callback ) ->
                                        callback (Io.field "oldVersion" event.message)

                                    _ ->
                                        promisified reply.id event
                            )
                            |> Io.andThen (\_ -> Io.await reply.id)
                            |> Io.map (Result.map (\_ -> ()))
            )


version : Database -> Io D.Value
version (Database db) =
    readValue db "version"


objectStoreNames : Database -> Io (List String)
objectStoreNames (Database db) =
    readNames db "objectStoreNames"


createObjectStore : Database -> String -> Maybe D.Value -> Io (Result String ObjectStore)
createObjectStore (Database db) storeName options =
    callMethod db "createObjectStore" (E.string storeName :: optionalArgument options) |> Io.map (Result.map ObjectStore)


deleteObjectStore : Database -> String -> Io (Result String ())
deleteObjectStore (Database db) storeName =
    callMethod db "deleteObjectStore" [ E.string storeName ] |> Io.map (Result.map (\_ -> ()))


{-| `stores` is a name or an array of names; without a mode the transaction is readonly.
-}
transaction : Database -> D.Value -> Maybe D.Value -> Io (Result String Transaction)
transaction (Database db) stores mode =
    Io.call "transaction" ([ ( "db", db ), ( "stores", stores ) ] ++ optionalField "mode" mode)
        |> Io.andThen
            (\reply ->
                case Io.objectResult reply of
                    Err threw ->
                        Io.succeed (Err threw)

                    Ok raw ->
                        cacheDone reply.id |> Io.map (\_ -> Ok (Transaction { raw = raw, done = Just reply.id }))
            )


close : Database -> Io ()
close (Database db) =
    Io.call "close" [ ( "db", db ) ] |> Io.map (\_ -> ())



-- TRANSACTION


{-| idb's `cacheDonePromiseForTransaction`, run when the transaction is wrapped: the first
`complete`, `error` or `abort` settles `done`, then all three are removed. A rejection is the
transaction's error, or a new AbortError when it has none: a request error bubbles to the
transaction before the transaction records it.
-}
cacheDone : Int -> Io ()
cacheDone id =
    let
        settleOnce settled =
            Io.settle id settled |> Io.andThen (\_ -> Io.unlisten id)

        failure message =
            D.decodeValue (D.field "txError" D.string) message |> Result.withDefault "AbortError"
    in
    Io.listen id
        (\event ->
            case event.kind of
                "complete" ->
                    settleOnce (Ok Nothing)

                "error" ->
                    settleOnce (Err (failure event.message))

                "abort" ->
                    settleOnce (Err (failure event.message))

                _ ->
                    Io.succeed ()
        )


done : Transaction -> Maybe (Promise ())
done (Transaction tx) =
    Maybe.map (\id -> Promise id (\_ -> ())) tx.done


{-| idb's `tx.store`: the only store in the scope, or `Nothing` when there are several. idb tests
the second name before it calls objectStore(), so on a finished transaction this fails with
InvalidStateError for one store and gives `Nothing` for several.
-}
store : Transaction -> Io (Result String (Maybe ObjectStore))
store (Transaction tx) =
    readNames tx.raw "objectStoreNames"
        |> Io.andThen
            (\names ->
                case names of
                    first :: second :: _ ->
                        if second /= "" then
                            Io.succeed (Ok Nothing)

                        else
                            objectStore (Transaction tx) first |> Io.map (Result.map Just)

                    [ only ] ->
                        objectStore (Transaction tx) only |> Io.map (Result.map Just)

                    [] ->
                        -- A transaction's scope is never empty: transaction() refuses that.
                        Io.succeed (Ok Nothing)
            )


objectStore : Transaction -> String -> Io (Result String ObjectStore)
objectStore (Transaction tx) storeName =
    callMethod tx.raw "objectStore" [ E.string storeName ] |> Io.map (Result.map ObjectStore)


abort : Transaction -> Io (Result String ())
abort (Transaction tx) =
    callMethod tx.raw "abort" [] |> Io.map (Result.map (\_ -> ()))



-- STORES AND INDEXES


storeSource : ObjectStore -> Source
storeSource =
    StoreSource


indexSource : Index -> Source
indexSource =
    IndexSource


index : ObjectStore -> String -> Io (Result String Index)
index (ObjectStore raw) indexName =
    callMethod raw "index" [ E.string indexName ] |> Io.map (Result.map Index)


createIndex : ObjectStore -> String -> D.Value -> Maybe D.Value -> Io (Result String Index)
createIndex (ObjectStore raw) indexName path options =
    callMethod raw "createIndex" (E.string indexName :: path :: optionalArgument options) |> Io.map (Result.map Index)


deleteIndex : ObjectStore -> String -> Io (Result String ())
deleteIndex (ObjectStore raw) indexName =
    callMethod raw "deleteIndex" [ E.string indexName ] |> Io.map (Result.map (\_ -> ()))


name : Source -> Io D.Value
name source =
    readValue (unwrap source) "name"


keyPath : Source -> Io D.Value
keyPath source =
    readValue (unwrap source) "keyPath"


autoIncrement : ObjectStore -> Io Bool
autoIncrement (ObjectStore raw) =
    readBool raw "autoIncrement"


indexNames : ObjectStore -> Io (List String)
indexNames (ObjectStore raw) =
    readNames raw "indexNames"


unique : Index -> Io Bool
unique (Index raw) =
    readBool raw "unique"


multiEntry : Index -> Io Bool
multiEntry (Index raw) =
    readBool raw "multiEntry"


{-| The request methods idb promisifies on a store or an index.
-}
type Method
    = Get
    | GetKey
    | GetAll
    | GetAllKeys
    | Count
    | Put
    | Add
    | Delete
    | Clear


methods : List Method
methods =
    [ Get, GetKey, GetAll, GetAllKeys, Count, Put, Add, Delete, Clear ]


methodName : Method -> String
methodName method =
    case method of
        Get ->
            "get"

        GetKey ->
            "getKey"

        GetAll ->
            "getAll"

        GetAllKeys ->
            "getAllKeys"

        Count ->
            "count"

        Put ->
            "put"

        Add ->
            "add"

        Delete ->
            "delete"

        Clear ->
            "clear"


methodOf : String -> Maybe Method
methodOf text =
    List.filter (\method -> methodName method == text) methods |> List.head


{-| database-extras' `writeMethods`; the others are its `readMethods`.
-}
isWrite : Method -> Bool
isWrite method =
    List.member method [ Put, Add, Delete, Clear ]


{-| Whether the first argument is a query: a key, or a key range.
-}
takesQuery : Method -> Bool
takesQuery method =
    not (List.member method [ Put, Add, Clear ])


{-| A wrapped request method: idb's `wrap(func.apply(unwrap(this), args))`.
-}
request : Method -> Source -> List D.Value -> Io (Result String (Promise (Maybe D.Value)))
request method source args =
    wrap (unwrap source) method args


openCursor : Source -> List D.Value -> Io (Result String (Promise (Maybe Cursor)))
openCursor source args =
    issue "request" [ ( "target", unwrap source ), ( "method", E.string "openCursor" ), ( "args", E.list identity args ) ] cursorResult


openKeyCursor : Source -> List D.Value -> Io (Result String (Promise (Maybe Cursor)))
openKeyCursor source args =
    issue "request" [ ( "target", unwrap source ), ( "method", E.string "openKeyCursor" ), ( "args", E.list identity args ) ] cursorResult



-- CURSORS


key : Cursor -> Io D.Value
key (Cursor raw) =
    readValue raw "key"


primaryKey : Cursor -> Io D.Value
primaryKey (Cursor raw) =
    readValue raw "primaryKey"


value : Cursor -> Io D.Value
value (Cursor raw) =
    readValue raw "value"


{-| idb's cursor advance methods: the host method, then the cursor's request promisified again,
which gives this cursor or `Nothing` past the end.
-}
move : String -> Cursor -> List D.Value -> Io (Result String (Promise (Maybe Cursor)))
move method (Cursor raw) args =
    issue "step" [ ( "cursor", raw ), ( "method", E.string method ), ( "args", E.list identity args ) ] cursorResult


advance : Cursor -> List D.Value -> Io (Result String (Promise (Maybe Cursor)))
advance =
    move "advance"


continue : Cursor -> List D.Value -> Io (Result String (Promise (Maybe Cursor)))
continue =
    move "continue"


continuePrimaryKey : Cursor -> List D.Value -> Io (Result String (Promise (Maybe Cursor)))
continuePrimaryKey =
    move "continuePrimaryKey"


update : Cursor -> D.Value -> Io (Result String (Promise (Maybe D.Value)))
update (Cursor raw) newValue =
    issue "request" [ ( "target", raw ), ( "method", E.string "update" ), ( "args", E.list identity [ newValue ] ) ] identity


delete : Cursor -> Io (Result String (Promise (Maybe D.Value)))
delete (Cursor raw) =
    issue "request" [ ( "target", raw ), ( "method", E.string "delete" ), ( "args", E.list identity [] ) ] identity



-- SHORTCUTS


{-| A database shortcut (database-extras' `getMethod`): a request method on the store, or with
an index (`getFromIndex`, ...) on one of its indexes. Only read methods have index forms.
-}
type Shortcut
    = Shortcut Method Bool


shortcutOf : String -> Maybe Shortcut
shortcutOf text =
    let
        suffix =
            "FromIndex"

        useIndex =
            String.endsWith suffix text

        target =
            if useIndex then
                String.dropRight (String.length suffix) text

            else
                text
    in
    case methodOf target of
        Just method ->
            if useIndex && isWrite method then
                Nothing

            else
                Just (Shortcut method useIndex)

        Nothing ->
            Nothing


shortcutMethod : Shortcut -> Method
shortcutMethod (Shortcut method _) =
    method


usesIndex : Shortcut -> Bool
usesIndex (Shortcut _ useIndex) =
    useIndex


{-| One shortcut call, as getMethod's async method: a transaction per call, `readwrite` for a
write, on `tx.store` or its index, and `Promise.all([request, isWrite && tx.done])`. The request
settles before its transaction does, so awaiting it and then `done` resolves and rejects as that
`Promise.all` does: a write resolves only after `done`, and the request's error comes first.
-}
shortcut : Shortcut -> Database -> String -> Maybe String -> List D.Value -> Io (Result String (Maybe D.Value))
shortcut (Shortcut method useIndex) db storeName indexName args =
    let
        mode =
            if isWrite method then
                "readwrite"

            else
                "readonly"

        target tx =
            store tx
                |> andThenOk
                    (\only ->
                        case ( only, useIndex, indexName ) of
                            ( Just found, True, Just named ) ->
                                index found named |> Io.map (Result.map IndexSource)

                            ( Just found, _, _ ) ->
                                Io.succeed (Ok (StoreSource found))

                            ( Nothing, _, _ ) ->
                                -- idb reads the method of an undefined `tx.store`.
                                Io.succeed (Err "TypeError")
                    )

        awaitDone tx =
            case done tx of
                Just finished ->
                    await finished

                Nothing ->
                    Io.succeed (Ok ())
    in
    transaction db (E.string storeName) (Just (E.string mode))
        |> andThenOk
            (\tx ->
                target tx
                    |> andThenOk (\source -> request method source args)
                    |> andThenOk await
                    |> andThenOk
                        (\resultValue ->
                            if isWrite method then
                                awaitDone tx |> Io.map (Result.map (\_ -> resultValue))

                            else
                                Io.succeed (Ok resultValue)
                        )
            )


andThenOk : (a -> Io (Result String b)) -> Io (Result String a) -> Io (Result String b)
andThenOk continueWith =
    Io.andThen
        (\outcome ->
            case outcome of
                Ok found ->
                    continueWith found

                Err error ->
                    Io.succeed (Err error)
        )



-- ITERATE


{-| idb's `iterate` generator. The first `next` opens `openCursor(...args)`; each later one awaits
the move the loop body made on the current cursor, or calls `continue()` when it made none. The
iterator is a value: each `next` and each move gives the iterator to use after it.
-}
type Iterator
    = Iterator
        { source : Source
        , args : List D.Value
        , cursor : Maybe Cursor
        , advanced : Maybe (Promise (Maybe Cursor))
        }


iterate : Source -> List D.Value -> Iterator
iterate source args =
    Iterator { source = source, args = args, cursor = Nothing, advanced = Nothing }


{-| The iterator at its next cursor, or `Nothing` when the iteration is over.
-}
next : Iterator -> Io (Result String (Maybe Iterator))
next (Iterator it) =
    let
        moved =
            case ( it.cursor, it.advanced ) of
                ( Nothing, _ ) ->
                    openCursor it.source it.args

                ( Just _, Just advanced ) ->
                    Io.succeed (Ok advanced)

                ( Just cursor, Nothing ) ->
                    continue cursor []
    in
    moved
        |> andThenOk await
        |> Io.map (Result.map (Maybe.map (\cursor -> Iterator { it | cursor = Just cursor, advanced = Nothing })))


current : Iterator -> Maybe Cursor
current (Iterator it) =
    it.cursor


{-| A move in the loop body (idb's proxied cursor): it calls the host method and records the
promise for the iterator instead of returning it.
-}
intercept : (Cursor -> List D.Value -> Io (Result String (Promise (Maybe Cursor)))) -> Iterator -> List D.Value -> Io (Result String Iterator)
intercept method (Iterator it) args =
    case it.cursor of
        Just cursor ->
            method cursor args |> Io.map (Result.map (\promise -> Iterator { it | advanced = Just promise }))

        Nothing ->
            Io.succeed (Ok (Iterator it))


iteratedAdvance : Iterator -> List D.Value -> Io (Result String Iterator)
iteratedAdvance =
    intercept advance


iteratedContinue : Iterator -> List D.Value -> Io (Result String Iterator)
iteratedContinue =
    intercept continue


iteratedContinuePrimaryKey : Iterator -> List D.Value -> Io (Result String Iterator)
iteratedContinuePrimaryKey =
    intercept continuePrimaryKey



-- KEY RANGES


{-| `IDBKeyRange[factory](...args)`: only, lowerBound, upperBound or bound.
-}
keyRange : String -> List D.Value -> Io (Result String D.Value)
keyRange factory args =
    Io.call "keyRange" [ ( "factory", E.string factory ), ( "args", E.list identity args ) ] |> Io.map Io.objectResult
