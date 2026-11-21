port module Main exposing (main)

import Json.Decode as D
import Json.Encode as E


port request : (E.Value -> msg) -> Sub msg


port response : E.Value -> Cmd msg


field key value =
    D.decodeValue (D.field key D.value) value |> Result.withDefault E.null


at i value =
    D.decodeValue (D.index i D.value) value |> Result.withDefault E.null


items value =
    D.decodeValue (D.list D.value) value |> Result.withDefault []


text value =
    D.decodeValue D.string value |> Result.withDefault ""


values =
    E.list identity


cmd name arg =
    values [ E.string name, arg ]


{-| One idb script (idb CONTRACT.md, "Input"): create the store, put every row in one readwrite
transaction, then walk the store with a cursor in primary-key order.
-}
persist : List E.Value -> E.Value
persist rows =
    let
        transaction mode step =
            E.object [ ( "op", E.string "tx" ), ( "conn", E.string "explorer" ), ( "stores", E.string "rows" ), ( "mode", E.string mode ), ( "steps", values [ step ] ), ( "catch", E.bool False ) ]

        schema =
            values
                [ E.object [ ( "op", E.string "createStore" ), ( "name", E.string "rows" ), ( "options", E.object [ ( "keyPath", E.string "id" ) ] ) ]
                , E.object [ ( "op", E.string "createIndex" ), ( "store", E.string "rows" ), ( "name", E.string "byGroup" ), ( "keyPath", E.string "group" ) ]
                ]

        openDb =
            E.object [ ( "op", E.string "openDB" ), ( "db", E.string "explorer" ), ( "conn", E.string "explorer" ), ( "version", E.int 1 ), ( "upgrade", schema ), ( "catch", E.bool False ) ]

        put row =
            E.object [ ( "op", E.string "put" ), ( "store", E.null ), ( "index", E.null ), ( "args", values [ row ] ), ( "catch", E.bool False ) ]

        write =
            E.object [ ( "op", E.string "batch" ), ( "requests", E.list put rows ), ( "abort", E.bool False ), ( "done", E.bool True ) ]

        walk =
            E.object [ ( "op", E.string "openCursor" ), ( "store", E.null ), ( "index", E.null ), ( "args", values [] ), ( "positions", values [] ), ( "rest", values [ values [ E.string "continue" ] ] ), ( "catch", E.bool False ) ]
    in
    E.object [ ( "steps", values [ openDb, transaction "readwrite" write, transaction "readonly" walk ] ) ]


{-| The stored rows' `value` texts, from the cursor entries of the idb log.
-}
stored history =
    field "log" (at 0 history)
        |> items
        |> List.filter (\entry -> text (at 0 entry) == "cursor")
        |> List.map (at 3 >> field "value")


hits history =
    field "hits" (at 1 (field "results" (at 1 history))) |> items


drive packet =
    let
        input =
            at 0 packet

        history =
            at 1 packet
    in
    case List.length (items history) of
        0 ->
            let
                row doc =
                    E.object [ ( "id", field "id" doc ), ( "group", E.string "documents" ), ( "value", E.string (E.encode 0 doc) ) ]
            in
            cmd "idb" (persist (List.map row (items (field "records" input))))

        1 ->
            let
                options =
                    E.object [ ( "fields", E.list E.string [ "title", "body" ] ), ( "storeFields", E.list E.string [ "body" ] ) ]

                documents =
                    stored history |> List.map (text >> D.decodeString D.value >> Result.withDefault E.null)

                add =
                    E.object [ ( "op", E.string "addAll" ), ( "documents", values documents ) ]

                query =
                    E.object [ ( "op", E.string "search" ), ( "query", field "query" input ) ]
            in
            cmd "minisearch" (E.object [ ( "options", options ), ( "operations", values [ add, query ] ) ])

        2 ->
            let
                markdown =
                    List.head (hits history) |> Maybe.map (field "body") |> Maybe.withDefault (E.string "")
            in
            cmd "marked" (E.object [ ( "op", E.string "parse" ), ( "markdown", markdown ) ])

        3 ->
            cmd "fflate" (E.object [ ( "op", E.string "strToU8" ), ( "text", E.string (String.join "\n" (List.map text (stored history))) ) ])

        4 ->
            let
                options =
                    E.object [ ( "level", E.int 6 ), ( "mtime", E.int 0 ) ]
            in
            cmd "fflate" (E.object [ ( "op", E.string "gzipSync" ), ( "data", E.object [ ( "bytes", at 3 history ) ] ), ( "options", options ) ])

        5 ->
            cmd "uplot" (field "chart" input)

        _ ->
            let
                ids =
                    E.list (field "id") (hits history)
            in
            values [ E.null, values [ ids, at 2 history, at 4 history, E.int (List.length (items (at 3 history))), at 5 history ] ]


main : Program () () E.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , subscriptions = \_ -> request identity
        , update = \packet _ -> ( (), response (drive packet) )
        }
