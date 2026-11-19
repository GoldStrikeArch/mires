port module Main exposing (main)

{-| The boundary of the Elm port (README "Port convention"), the Elm port of
`libraries/minisearch/reference.mjs`. The JS host (`js/index.js`) sends the corpus input through
the `request` port; it is decoded with `Json.Decode` into the port's types, the way the adapter
maps JSON onto the MiniSearch API: options and queries into `SearchOptions` and `Query`, the
declarative `filter`, `boostDocument` and `boostTerm` into the callbacks they describe, documents
into the values MiniSearch reads from them. The operation script then runs on one index, and the
result is encoded with `Json.Encode`, keys in the adapter's order, and answered through the
`response` port.

-}

import Dict exposing (Dict)
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import MiniSearch exposing (Document, Id(..), MiniSearch, Query(..), SearchOptions, Value(..))
import Platform


port request : (Decode.Value -> msg) -> Sub msg


port response : Encode.Value -> Cmd msg


main : Program () () Decode.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \value model -> ( model, response (handle value) )
        , subscriptions = \_ -> request identity
        }


handle : Decode.Value -> Encode.Value
handle value =
    case Decode.decodeValue scriptDecoder value of
        Ok ( options, operations ) ->
            Encode.object [ ( "ok", run options operations ) ]

        Err error ->
            Encode.object [ ( "error", Encode.string (Decode.errorToString error) ) ]



-- RUNNING THE SCRIPT


type Operation
    = Add Document
    | AddAll (List Document)
    | Remove Document
    | RemoveAll MiniSearch.RemoveAllArgument
    | Discard Id
    | DiscardAll (List Id)
    | Replace Document
    | Vacuum MiniSearch.VacuumOptions
    | Search Query SearchOptions
    | AutoSuggest String SearchOptions
    | Has Id
    | GetStoredFields Id
    | Stats
    | ToJson
    | Unknown String


{-| One `new MiniSearch(options)`, then one record per operation: `op`, the payload when the
operation returned, `warnings` when MiniSearch logged, `error` when it threw (the script then goes
on with the index as MiniSearch left it).
-}
run : MiniSearch.Options -> List ( String, Operation ) -> Encode.Value
run options operations =
    let
        step ( op, operation ) ( index, records ) =
            let
                ( after, outcome ) =
                    execute operation (MiniSearch.clearWarnings index)

                logged =
                    case MiniSearch.warnings after of
                        [] ->
                            []

                        messages ->
                            [ ( "warnings", Encode.list Encode.string messages ) ]

                fields =
                    case outcome of
                        Ok payload ->
                            payload ++ logged

                        Err message ->
                            logged ++ [ ( "error", Encode.string message ) ]
            in
            ( after, Encode.object (( "op", Encode.string op ) :: fields) :: records )

        ( _, results ) =
            List.foldl step ( MiniSearch.new options, [] ) operations
    in
    Encode.object [ ( "results", Encode.list identity (List.reverse results) ) ]


type alias Payload =
    List ( String, Encode.Value )


execute : Operation -> MiniSearch -> ( MiniSearch, Result String Payload )
execute operation index =
    case operation of
        Add document ->
            orUnchanged index (MiniSearch.add document index)

        AddAll documents ->
            noPayload (MiniSearch.addAll documents index)

        Remove document ->
            orUnchanged index (MiniSearch.remove document index)

        RemoveAll argument ->
            noPayload (MiniSearch.removeAll argument index)

        Discard id ->
            orUnchanged index (MiniSearch.discard id index)

        DiscardAll ids ->
            noPayload (MiniSearch.discardAll ids index)

        Replace document ->
            noPayload (MiniSearch.replace document index)

        Vacuum options ->
            -- The Promise it returns settles after the synchronous run (CONTRACT.md, "Vacuuming").
            ( MiniSearch.vacuum options index, Ok [] )

        Search query options ->
            MiniSearch.search query options index
                |> Tuple.mapSecond (Result.map (\hits -> [ ( "hits", Encode.list encodeHit hits ) ]))

        AutoSuggest query options ->
            MiniSearch.autoSuggest query options index
                |> Tuple.mapSecond (Result.map (\suggestions -> [ ( "suggestions", Encode.list encodeSuggestion suggestions ) ]))

        Has id ->
            ( index, Ok [ ( "value", Encode.bool (MiniSearch.has id index) ) ] )

        GetStoredFields id ->
            ( index
            , Ok
                [ ( "value"
                  , case MiniSearch.getStoredFields id index of
                        Just stored ->
                            encodeStored encodeValue stored

                        Nothing ->
                            Encode.null
                  )
                ]
            )

        Stats ->
            ( index
            , Ok
                [ ( "documentCount", Encode.int (MiniSearch.documentCount index) )
                , ( "termCount", Encode.int (MiniSearch.termCount index) )
                , ( "dirtCount", Encode.int (MiniSearch.dirtCount index) )
                , ( "dirtFactor", Encode.float (MiniSearch.dirtFactor index) )
                , ( "isVacuuming", Encode.bool (MiniSearch.isVacuuming index) )
                ]
            )

        ToJson ->
            ( index, Ok [ ( "value", encodeSerialized (MiniSearch.toJson index) ) ] )

        Unknown op ->
            -- The adapter's own error for an operation it does not know.
            ( index, Err ("unknown operation " ++ op) )


orUnchanged : MiniSearch -> Result String MiniSearch -> ( MiniSearch, Result String Payload )
orUnchanged index result =
    case result of
        Ok next ->
            ( next, Ok [] )

        Err message ->
            ( index, Err message )


noPayload : ( MiniSearch, Result String () ) -> ( MiniSearch, Result String Payload )
noPayload =
    Tuple.mapSecond (Result.map (always []))



-- DECODING


scriptDecoder : Decoder ( MiniSearch.Options, List ( String, Operation ) )
scriptDecoder =
    Decode.field "options" indexOptionsDecoder
        |> Decode.andThen
            (\options ->
                Decode.field "operations" (Decode.list (operationDecoder (documentNames options)))
                    |> Decode.map (Tuple.pair options)
            )


{-| The names MiniSearch reads from a document.
-}
documentNames : MiniSearch.Options -> List String
documentNames options =
    options.idField :: options.fields ++ options.storeFields


indexOptionsDecoder : Decoder MiniSearch.Options
indexOptionsDecoder =
    Decode.succeed MiniSearch.Options
        |> required "fields" (Decode.list Decode.string)
        |> withDefault "idField" Decode.string "id"
        |> withDefault "storeFields" (Decode.list Decode.string) []
        |> withDefault "searchOptions" searchOptionsDecoder MiniSearch.noSearchOptions
        |> withDefault "autoSuggestOptions" searchOptionsDecoder MiniSearch.noSearchOptions
        |> withDefault "autoVacuum" autoVacuumDecoder (MiniSearch.AutoVacuumFlag True)


autoVacuumDecoder : Decoder MiniSearch.AutoVacuum
autoVacuumDecoder =
    Decode.oneOf
        [ Decode.map MiniSearch.AutoVacuumFlag Decode.bool
        , Decode.succeed MiniSearch.AutoVacuumOptions
            |> optional "batchSize" Decode.int
            |> optional "minDirtCount" Decode.float
            |> optional "minDirtFactor" Decode.float
            |> Decode.map MiniSearch.AutoVacuumWith
        ]


searchOptionsDecoder : Decoder SearchOptions
searchOptionsDecoder =
    Decode.succeed SearchOptions
        |> optional "fields" (Decode.list Decode.string)
        |> optional "filter" filterDecoder
        |> optional "boost" (Decode.dict Decode.float)
        |> optional "weights"
            (Decode.succeed MiniSearch.Weights
                |> optional "fuzzy" Decode.float
                |> optional "prefix" Decode.float
            )
        |> optional "boostDocument" (pairs idDecoder |> Decode.map (\factors documentId _ _ -> factor factors documentId))
        |> optional "boostTerm" (pairs Decode.string |> Decode.map (\factors term _ _ -> factor factors term))
        |> optional "prefix" (Decode.map MiniSearch.PrefixFlag Decode.bool)
        |> optional "fuzzy"
            (Decode.oneOf
                [ Decode.map MiniSearch.FuzzyFlag Decode.bool
                , Decode.map MiniSearch.FuzzyNumber Decode.float
                ]
            )
        |> optional "maxFuzzy" Decode.int
        |> optional "combineWith" Decode.string
        |> optional "bm25"
            (Decode.map3 MiniSearch.Bm25
                (Decode.field "k" Decode.float)
                (Decode.field "b" Decode.float)
                (Decode.field "d" Decode.float)
            )


{-| `filter: { field, in: [values] }` is `(result) => values.includes(result[field])`.
-}
filterDecoder : Decoder (MiniSearch.SearchResult -> Bool)
filterDecoder =
    Decode.map2
        (\field values result ->
            case MiniSearch.resultField field result of
                Just found ->
                    List.member found values

                Nothing ->
                    False
        )
        (Decode.field "field" Decode.string)
        (Decode.field "in" (Decode.list valueDecoder))


{-| `[[key, factor], ...]`.
-}
pairs : Decoder a -> Decoder (List ( a, Float ))
pairs key =
    Decode.list (Decode.map2 Tuple.pair (Decode.index 0 key) (Decode.index 1 Decode.float))


{-| The factor of the first pair whose key is `key` (`===`), else 1.
-}
factor : List ( a, Float ) -> a -> Float
factor factors key =
    case factors of
        [] ->
            1

        ( candidate, value ) :: rest ->
            if candidate == key then
                value

            else
                factor rest key


{-| A string, `{ wildcard: true }` (`MiniSearch.wildcard`, at the top or as a subquery), or a
combination: search options and `queries`.
-}
queryDecoder : Decoder Query
queryDecoder =
    Decode.oneOf
        [ Decode.map Terms Decode.string
        , Decode.field "wildcard" Decode.bool
            |> Decode.andThen
                (\wildcard ->
                    if wildcard then
                        Decode.succeed Wildcard

                    else
                        Decode.fail "not a wildcard"
                )
        , Decode.map2 Combination searchOptionsDecoder (Decode.field "queries" (Decode.list (Decode.lazy (\_ -> queryDecoder))))
        ]


operationDecoder : List String -> Decoder ( String, Operation )
operationDecoder names =
    Decode.field "op" Decode.string
        |> Decode.andThen (\op -> Decode.map (Tuple.pair op) (operationArguments names op))


operationArguments : List String -> String -> Decoder Operation
operationArguments names op =
    let
        document =
            documentDecoder names
    in
    case op of
        "add" ->
            Decode.map Add (Decode.field "document" document)

        "addAll" ->
            Decode.map AddAll (Decode.field "documents" (Decode.list document))

        "remove" ->
            Decode.map Remove (Decode.field "document" document)

        "removeAll" ->
            -- `removeAll()` without the key; `removeAll(null)` with an explicit null.
            optionalField "documents" (Decode.nullable (Decode.list document))
                |> Decode.map
                    (\argument ->
                        case argument of
                            Nothing ->
                                RemoveAll MiniSearch.NoArgument

                            Just documents ->
                                RemoveAll (MiniSearch.Argument documents)
                    )

        "discard" ->
            Decode.map Discard (Decode.field "id" idDecoder)

        "discardAll" ->
            Decode.map DiscardAll (Decode.field "ids" (Decode.list idDecoder))

        "replace" ->
            Decode.map Replace (Decode.field "document" document)

        "vacuum" ->
            Decode.map Vacuum
                (fieldOr "options"
                    (Decode.map MiniSearch.VacuumOptions (optionalField "batchSize" Decode.int))
                    { batchSize = Nothing }
                )

        "search" ->
            Decode.map2 Search
                (Decode.field "query" queryDecoder)
                (fieldOr "options" searchOptionsDecoder MiniSearch.noSearchOptions)

        "autoSuggest" ->
            Decode.map2 AutoSuggest
                (Decode.field "query" Decode.string)
                (fieldOr "options" searchOptionsDecoder MiniSearch.noSearchOptions)

        "has" ->
            Decode.map Has (Decode.field "id" idDecoder)

        "getStoredFields" ->
            Decode.map GetStoredFields (Decode.field "id" idDecoder)

        "stats" ->
            Decode.succeed Stats

        "toJSON" ->
            Decode.succeed ToJson

        _ ->
            Decode.succeed (Unknown op)


{-| A document, as the values of `names` (`document[name]`: absent names are left out).
-}
documentDecoder : List String -> Decoder Document
documentDecoder names =
    List.foldl
        (\name decoder ->
            Decode.map2
                (\found document ->
                    case found of
                        Just v ->
                            Dict.insert name v document

                        Nothing ->
                            document
                )
                (optionalField name valueDecoder)
                decoder
        )
        (Decode.succeed Dict.empty)
        names


{-| A document value inside the parity domain: a string, a number or null.
-}
valueDecoder : Decoder Value
valueDecoder =
    Decode.oneOf
        [ Decode.map MiniSearch.Text Decode.string
        , Decode.map MiniSearch.Number Decode.float
        , Decode.null MiniSearch.Null
        ]


idDecoder : Decoder Id
idDecoder =
    Decode.oneOf
        [ Decode.map StringId Decode.string
        , Decode.map NumberId Decode.float
        ]


{-| A field that may be absent; when present it must decode (an invalid value is an error, not an
absent field).
-}
optionalField : String -> Decoder a -> Decoder (Maybe a)
optionalField name decoder =
    Decode.maybe (Decode.field name Decode.value)
        |> Decode.andThen
            (\found ->
                case found of
                    Just _ ->
                        Decode.map Just (Decode.field name decoder)

                    Nothing ->
                        Decode.succeed Nothing
            )


{-| An optional field's value, or `fallback` when it is absent.
-}
fieldOr : String -> Decoder a -> a -> Decoder a
fieldOr name decoder fallback =
    optionalField name decoder |> Decode.map (Maybe.withDefault fallback)


required : String -> Decoder a -> Decoder (a -> b) -> Decoder b
required name decoder =
    Decode.map2 (|>) (Decode.field name decoder)


optional : String -> Decoder a -> Decoder (Maybe a -> b) -> Decoder b
optional name decoder =
    Decode.map2 (|>) (optionalField name decoder)


withDefault : String -> Decoder a -> a -> Decoder (a -> b) -> Decoder b
withDefault name decoder fallback =
    Decode.map2 (|>) (fieldOr name decoder fallback)



-- ENCODING


encodeId : Id -> Encode.Value
encodeId documentId =
    case documentId of
        StringId text ->
            Encode.string text

        NumberId number ->
            Encode.float number


encodeValue : Value -> Encode.Value
encodeValue v =
    case v of
        MiniSearch.Text text ->
            Encode.string text

        MiniSearch.Number number ->
            Encode.float number

        MiniSearch.Null ->
            Encode.null


encodeStored : (Value -> Encode.Value) -> MiniSearch.StoredFields -> Encode.Value
encodeStored encode stored =
    Encode.object (List.map (Tuple.mapSecond encode) stored)


strings : List String -> Encode.Value
strings =
    Encode.list Encode.string


{-| The adapter's copy of a result: its keys in order, `match` as `[term, fields]` pairs, then the
stored fields as `Object.assign` added them. `Encode.object` sets the keys in order on a plain
object, so a stored field named `id` replaces the id in place, exactly as `Object.assign` does.
-}
encodeHit : MiniSearch.SearchResult -> Encode.Value
encodeHit hit =
    Encode.object
        ([ ( "id", encodeId hit.id )
         , ( "score", Encode.float hit.score )
         , ( "terms", strings hit.terms )
         , ( "queryTerms", strings hit.queryTerms )
         , ( "match", Encode.list (\( term, fields ) -> Encode.list identity [ Encode.string term, strings fields ]) hit.match )
         ]
            ++ List.map (Tuple.mapSecond encodeValue) hit.storedFields
        )


encodeSuggestion : MiniSearch.Suggestion -> Encode.Value
encodeSuggestion suggestion =
    Encode.object
        [ ( "suggestion", Encode.string suggestion.suggestion )
        , ( "terms", strings suggestion.terms )
        , ( "score", Encode.float suggestion.score )
        ]


{-| A number as `JSON.parse(JSON.stringify(number))` gives it back: non-finite numbers are `null`,
`-0` is `0`.
-}
jsonNumber : Float -> Encode.Value
jsonNumber number =
    if isNaN number || isInfinite number then
        Encode.null

    else if number == 0 then
        Encode.float 0

    else
        Encode.float number


jsonValue : Value -> Encode.Value
jsonValue v =
    case v of
        MiniSearch.Number number ->
            jsonNumber number

        _ ->
            encodeValue v


{-| `JSON.parse(JSON.stringify(index))`. Integer keys are set in ascending order, which is also the
order a JS object lists them in.
-}
encodeSerialized : MiniSearch.Serialized -> Encode.Value
encodeSerialized serialized =
    let
        byShortId encode entries =
            Encode.object (List.map (\( shortId, entry ) -> ( String.fromInt shortId, encode entry )) entries)

        holes encode =
            Encode.list
                (\entry ->
                    case entry of
                        Just v ->
                            encode v

                        Nothing ->
                            Encode.null
                )
    in
    Encode.object
        [ ( "documentCount", Encode.int serialized.documentCount )
        , ( "nextId", Encode.int serialized.nextId )
        , ( "documentIds", byShortId (jsonValue << idValue) serialized.documentIds )
        , ( "fieldIds", Encode.object (List.map (Tuple.mapSecond Encode.int) serialized.fieldIds) )
        , ( "fieldLength", byShortId (holes Encode.int) serialized.fieldLength )
        , ( "averageFieldLength", holes jsonNumber serialized.averageFieldLength )
        , ( "storedFields", byShortId (encodeStored jsonValue) serialized.storedFields )
        , ( "dirtCount", Encode.int serialized.dirtCount )
        , ( "index"
          , Encode.list
                (\( term, fields ) ->
                    Encode.list identity [ Encode.string term, byShortId (byShortId Encode.int) fields ]
                )
                serialized.index
          )
        , ( "serializationVersion", Encode.int serialized.serializationVersion )
        ]


idValue : Id -> Value
idValue documentId =
    case documentId of
        StringId text ->
            MiniSearch.Text text

        NumberId number ->
            MiniSearch.Number number
