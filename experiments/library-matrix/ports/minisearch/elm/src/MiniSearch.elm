module MiniSearch exposing
    ( MiniSearch, Options, AutoVacuum(..), AutoVacuumOptions, VacuumOptions
    , SearchOptions, noSearchOptions, Prefix(..), Fuzzy(..), Weights, Bm25
    , Query(..), SearchResult, Suggestion, resultField
    , Document, Value(..), Id(..), idToString, StoredFields, RemoveAllArgument(..)
    , new, add, addAll, remove, removeAll, discard, discardAll, replace, vacuum
    , search, autoSuggest, has, getStoredFields
    , documentCount, termCount, dirtCount, dirtFactor, isVacuuming
    , Serialized, toJson
    , clearWarnings, warnings
    )

{-| MiniSearch 7.2.0 (`minisearch/dist/es/index.js`), the subset of
`libraries/minisearch/CONTRACT.md`: the `MiniSearch` class, method by method, over a persistent
index. Every method that changes the index takes it and returns the new one; a method that throws
in MiniSearch returns `Err` with MiniSearch's message, together with the index as MiniSearch leaves
it when it throws part way (`addAll`, `discardAll`, `replace`, and `search`, whose cleanup of
discarded postings happens before an invalid operator is found).

The logger records MiniSearch's warnings in the index (`warnings`), in order.

-}

import Dict exposing (Dict)
import OrderedDict exposing (OrderedDict)
import SearchableMap exposing (SearchableMap)
import Set
import Text



-- DOCUMENTS


{-| A value MiniSearch reads from a document (inside the parity domain: a string, a number or
null).
-}
type Value
    = Text String
    | Number Float
    | Null


{-| A document: the values of the names MiniSearch reads from it (`idField`, `fields`,
`storeFields`); a missing name is an absent (`undefined`) value.
-}
type alias Document =
    Dict String Value


{-| A document id. Ids are the keys of a `Map`: `1` and `"1"` are different documents.
-}
type Id
    = StringId String
    | NumberId Float


{-| An id as a template literal writes it: a number in decimal, a string as itself.
-}
idToString : Id -> String
idToString id =
    case id of
        StringId text ->
            text

        NumberId number ->
            String.fromFloat number


{-| A document's stored fields, in `storeFields` order.
-}
type alias StoredFields =
    List ( String, Value )



-- OPTIONS


type alias Options =
    { fields : List String
    , idField : String
    , storeFields : List String
    , searchOptions : SearchOptions
    , autoSuggestOptions : SearchOptions
    , autoVacuum : AutoVacuum
    }


{-| `autoVacuum: true | false | { batchSize, minDirtCount, minDirtFactor }`.
-}
type AutoVacuum
    = AutoVacuumFlag Bool
    | AutoVacuumWith AutoVacuumOptions


type alias AutoVacuumOptions =
    { batchSize : Maybe Int, minDirtCount : Maybe Float, minDirtFactor : Maybe Float }


type alias VacuumOptions =
    { batchSize : Maybe Int }


type alias VacuumConditions =
    { minDirtCount : Maybe Float, minDirtFactor : Maybe Float }


{-| Search options. Every key is optional; merging two of them is object spread: a key present
in the second replaces the first's value whole (`merge`).
-}
type alias SearchOptions =
    { fields : Maybe (List String)
    , filter : Maybe (SearchResult -> Bool)
    , boost : Maybe (Dict String Float)
    , weights : Maybe Weights
    , boostDocument : Maybe (Id -> String -> Maybe StoredFields -> Float)
    , boostTerm : Maybe (String -> Int -> List String -> Float)
    , prefix : Maybe Prefix
    , fuzzy : Maybe Fuzzy
    , maxFuzzy : Maybe Int
    , combineWith : Maybe String
    , bm25 : Maybe Bm25
    }


{-| `prefix: boolean | (term, i, terms) => boolean` (the function form is autoSuggest's default).
-}
type Prefix
    = PrefixFlag Bool
    | PrefixWhen (String -> Int -> List String -> Bool)


{-| `fuzzy: boolean | number`.
-}
type Fuzzy
    = FuzzyFlag Bool
    | FuzzyNumber Float


type alias Weights =
    { fuzzy : Maybe Float, prefix : Maybe Float }


type alias Bm25 =
    { k : Float, b : Float, d : Float }


{-| `{}`.
-}
noSearchOptions : SearchOptions
noSearchOptions =
    { fields = Nothing
    , filter = Nothing
    , boost = Nothing
    , weights = Nothing
    , boostDocument = Nothing
    , boostTerm = Nothing
    , prefix = Nothing
    , fuzzy = Nothing
    , maxFuzzy = Nothing
    , combineWith = Nothing
    , bm25 = Nothing
    }


{-| `{ ...base, ...options }`.
-}
merge : SearchOptions -> SearchOptions -> SearchOptions
merge base options =
    { fields = orElse base.fields options.fields
    , filter = orElse base.filter options.filter
    , boost = orElse base.boost options.boost
    , weights = orElse base.weights options.weights
    , boostDocument = orElse base.boostDocument options.boostDocument
    , boostTerm = orElse base.boostTerm options.boostTerm
    , prefix = orElse base.prefix options.prefix
    , fuzzy = orElse base.fuzzy options.fuzzy
    , maxFuzzy = orElse base.maxFuzzy options.maxFuzzy
    , combineWith = orElse base.combineWith options.combineWith
    , bm25 = orElse base.bm25 options.bm25
    }


orElse : Maybe a -> Maybe a -> Maybe a
orElse base value =
    case value of
        Just _ ->
            value

        Nothing ->
            base


defaultBm25 : Bm25
defaultBm25 =
    { k = 1.2, b = 0.7, d = 0.5 }


defaultSearchOptions : SearchOptions
defaultSearchOptions =
    { noSearchOptions
        | combineWith = Just operatorOr
        , prefix = Just (PrefixFlag False)
        , fuzzy = Just (FuzzyFlag False)
        , maxFuzzy = Just 6
        , boost = Just Dict.empty
        , weights = Just { fuzzy = Just 0.45, prefix = Just 0.375 }
        , bm25 = Just defaultBm25
    }


defaultAutoSuggestOptions : SearchOptions
defaultAutoSuggestOptions =
    { noSearchOptions
        | combineWith = Just operatorAnd
        , prefix = Just (PrefixWhen (\_ i terms -> i == List.length terms - 1))
    }


defaultAutoVacuumOptions : AutoVacuumOptions
defaultAutoVacuumOptions =
    { batchSize = Just defaultBatchSize, minDirtCount = Just defaultMinDirtCount, minDirtFactor = Just defaultMinDirtFactor }


defaultBatchSize : Int
defaultBatchSize =
    1000


defaultMinDirtCount : Float
defaultMinDirtCount =
    20


defaultMinDirtFactor : Float
defaultMinDirtFactor =
    0.1


operatorOr : String
operatorOr =
    "or"


operatorAnd : String
operatorAnd =
    "and"


operatorAndNot : String
operatorAndNot =
    "and_not"



-- THE INDEX


{-| A term's postings: field id to posting list, a posting list being short id to term
frequency. The order of a term's fields is never observable (serialization lists them by id), and
a posting list's insertion order is ascending short id (a document only ever adds postings under
the newest short id), which is `Dict`'s order.
-}
type alias FieldsData =
    Dict Int (Dict Int Int)


type alias MiniSearch =
    { options : Options
    , index : SearchableMap FieldsData
    , documentCount : Int
    , documentIds : Dict Int Id
    , idToShortId : IdTable
    , fieldIds : Dict String Int
    , fieldLength : Dict Int (Dict Int Int)
    , avgFieldLength : Dict Int Float
    , nextId : Int
    , storedFields : Dict Int StoredFields
    , dirtCount : Int
    , currentVacuum : Bool
    , log : List String
    }


{-| `new MiniSearch(options)`.
-}
new : Options -> MiniSearch
new options =
    { options =
        { options
            | autoVacuum =
                case options.autoVacuum of
                    AutoVacuumFlag True ->
                        AutoVacuumWith defaultAutoVacuumOptions

                    other ->
                        other
            , searchOptions = merge defaultSearchOptions options.searchOptions
            , autoSuggestOptions = merge defaultAutoSuggestOptions options.autoSuggestOptions
        }
    , index = SearchableMap.empty
    , documentCount = 0
    , documentIds = Dict.empty
    , idToShortId = emptyIdTable
    , fieldIds = Dict.fromList (List.indexedMap (\i field -> ( field, i )) options.fields)
    , fieldLength = Dict.empty
    , avgFieldLength = Dict.empty
    , nextId = 0
    , storedFields = Dict.empty
    , dirtCount = 0
    , currentVacuum = False
    , log = []
    }


{-| `_idToShortId`: a `Map` keyed by id. Its order is never observable, so it is two `Dict`s.
-}
type alias IdTable =
    { strings : Dict String Int, numbers : Dict Float Int }


emptyIdTable : IdTable
emptyIdTable =
    { strings = Dict.empty, numbers = Dict.empty }


shortIdOf : Id -> IdTable -> Maybe Int
shortIdOf id table =
    case id of
        StringId text ->
            Dict.get text table.strings

        NumberId number ->
            Dict.get number table.numbers


insertId : Id -> Int -> IdTable -> IdTable
insertId id shortId table =
    case id of
        StringId text ->
            { table | strings = Dict.insert text shortId table.strings }

        NumberId number ->
            { table | numbers = Dict.insert number shortId table.numbers }


removeId : Id -> IdTable -> IdTable
removeId id table =
    case id of
        StringId text ->
            { table | strings = Dict.remove text table.strings }

        NumberId number ->
            { table | numbers = Dict.remove number table.numbers }


{-| `document[idField]`, `null` or absent being no id.
-}
documentId : String -> Document -> Maybe Id
documentId idField document =
    case Dict.get idField document of
        Just (Text text) ->
            Just (StringId text)

        Just (Number number) ->
            Just (NumberId number)

        _ ->
            Nothing


{-| The text MiniSearch indexes for a field, `stringifyField(document[name])`
(`fieldValue.toString()`); a `null` or absent value is skipped.
-}
fieldText : String -> Document -> Maybe String
fieldText name document =
    case Dict.get name document of
        Just (Text text) ->
            Just text

        Just (Number number) ->
            Just (String.fromFloat number)

        Just Null ->
            Nothing

        Nothing ->
            Nothing



-- WARNINGS


warn : String -> MiniSearch -> MiniSearch
warn message miniSearch =
    { miniSearch | log = message :: miniSearch.log }


{-| The warnings logged since the last `clearWarnings`, in order.
-}
warnings : MiniSearch -> List String
warnings miniSearch =
    List.reverse miniSearch.log


clearWarnings : MiniSearch -> MiniSearch
clearWarnings miniSearch =
    { miniSearch | log = [] }



-- ADDING


{-| `add(document)`.
-}
add : Document -> MiniSearch -> Result String MiniSearch
add document miniSearch =
    let
        idField =
            miniSearch.options.idField
    in
    case documentId idField document of
        Nothing ->
            Err ("MiniSearch: document does not have ID field \"" ++ idField ++ "\"")

        Just id ->
            if isJust (shortIdOf id miniSearch.idToShortId) then
                Err ("MiniSearch: duplicate ID " ++ idToString id)

            else
                let
                    shortId =
                        miniSearch.nextId

                    withId =
                        saveStoredFields shortId
                            document
                            { miniSearch
                                | idToShortId = insertId id shortId miniSearch.idToShortId
                                , documentIds = Dict.insert shortId id miniSearch.documentIds
                                , documentCount = miniSearch.documentCount + 1
                                , nextId = miniSearch.nextId + 1
                            }
                in
                Ok (List.foldl (addField document shortId) withId (List.indexedMap Tuple.pair miniSearch.options.fields))


addField : Document -> Int -> ( Int, String ) -> MiniSearch -> MiniSearch
addField document shortId ( fieldId, field ) miniSearch =
    case fieldText field document of
        Just text ->
            indexField fieldId shortId text miniSearch

        Nothing ->
            miniSearch


indexField : Int -> Int -> String -> MiniSearch -> MiniSearch
indexField fieldId shortId text miniSearch =
    let
        tokens =
            Text.tokenize text

        withLength =
            addFieldLength shortId fieldId (miniSearch.documentCount - 1) (uniqueCount tokens) miniSearch

        addToken token index =
            case Text.processTerm token of
                "" ->
                    index

                term ->
                    addTerm fieldId shortId term index
    in
    { withLength | index = List.foldl addToken withLength.index tokens }


{-| `new Set(tokens).size`: distinct raw tokens, before lower-casing.
-}
uniqueCount : List String -> Int
uniqueCount tokens =
    Set.size (Set.fromList tokens)


{-| `addTerm`: one more occurrence of `term` in the field of the document.
-}
addTerm : Int -> Int -> String -> SearchableMap FieldsData -> SearchableMap FieldsData
addTerm fieldId shortId term index =
    SearchableMap.update term
        (\data ->
            let
                fields =
                    Maybe.withDefault Dict.empty data
            in
            case Dict.get fieldId fields of
                Nothing ->
                    Dict.insert fieldId (Dict.singleton shortId 1) fields

                Just postings ->
                    Dict.insert fieldId (Dict.insert shortId (Maybe.withDefault 0 (Dict.get shortId postings) + 1) postings) fields
        )
        index


{-| `addAll(documents)`: the first error ends it, keeping the documents added before.
-}
addAll : List Document -> MiniSearch -> ( MiniSearch, Result String () )
addAll documents miniSearch =
    each add documents miniSearch


each : (a -> MiniSearch -> Result String MiniSearch) -> List a -> MiniSearch -> ( MiniSearch, Result String () )
each operation items miniSearch =
    case items of
        [] ->
            ( miniSearch, Ok () )

        item :: rest ->
            case operation item miniSearch of
                Ok next ->
                    each operation rest next

                Err message ->
                    ( miniSearch, Err message )


saveStoredFields : Int -> Document -> MiniSearch -> MiniSearch
saveStoredFields shortId document miniSearch =
    case miniSearch.options.storeFields of
        [] ->
            miniSearch

        storeFields ->
            let
                stored =
                    List.filterMap (\name -> Maybe.map (Tuple.pair name) (Dict.get name document)) storeFields
            in
            { miniSearch | storedFields = Dict.insert shortId stored miniSearch.storedFields }



-- FIELD LENGTHS


{-| `addFieldLength`: `fieldLength[shortId][fieldId] = length` (an array with holes for the absent
fields), and the average becomes `(average * count + length) / (count + 1)`, starting from
`avgFieldLength[fieldId] || 0`.
-}
addFieldLength : Int -> Int -> Int -> Int -> MiniSearch -> MiniSearch
addFieldLength shortId fieldId count length miniSearch =
    let
        average =
            case Dict.get fieldId miniSearch.avgFieldLength of
                Just value ->
                    if isNaN value then
                        0

                    else
                        value

                Nothing ->
                    0

        total =
            average * toFloat count + toFloat length
    in
    { miniSearch
        | fieldLength =
            Dict.insert shortId
                (Dict.insert fieldId length (Maybe.withDefault Dict.empty (Dict.get shortId miniSearch.fieldLength)))
                miniSearch.fieldLength
        , avgFieldLength = Dict.insert fieldId (total / toFloat (count + 1)) miniSearch.avgFieldLength
    }


{-| `removeFieldLength`. A hole read here is `undefined`, so the average becomes NaN.
-}
removeFieldLength : Int -> Int -> Int -> MiniSearch -> MiniSearch
removeFieldLength fieldId count length miniSearch =
    let
        average =
            if count == 1 then
                0

            else
                let
                    current =
                        Maybe.withDefault (0 / 0) (Dict.get fieldId miniSearch.avgFieldLength)
                in
                (current * toFloat count - toFloat length) / toFloat (count - 1)
    in
    { miniSearch | avgFieldLength = Dict.insert fieldId average miniSearch.avgFieldLength }



-- REMOVING


{-| `remove(document)`: the document as given must be the one that was added; nothing checks it,
and a term it no longer has is a `version_conflict` warning.
-}
remove : Document -> MiniSearch -> Result String MiniSearch
remove document miniSearch =
    let
        idField =
            miniSearch.options.idField
    in
    case documentId idField document of
        Nothing ->
            Err ("MiniSearch: document does not have ID field \"" ++ idField ++ "\"")

        Just id ->
            case shortIdOf id miniSearch.idToShortId of
                Nothing ->
                    Err ("MiniSearch: cannot remove document with ID " ++ idToString id ++ ": it is not in the index")

                Just shortId ->
                    let
                        removed =
                            List.foldl (removeField document shortId) miniSearch (List.indexedMap Tuple.pair miniSearch.options.fields)
                    in
                    Ok
                        { removed
                            | storedFields = Dict.remove shortId removed.storedFields
                            , documentIds = Dict.remove shortId removed.documentIds
                            , idToShortId = removeId id removed.idToShortId
                            , fieldLength = Dict.remove shortId removed.fieldLength
                            , documentCount = removed.documentCount - 1
                        }


removeField : Document -> Int -> ( Int, String ) -> MiniSearch -> MiniSearch
removeField document shortId ( fieldId, field ) miniSearch =
    case fieldText field document of
        Just text ->
            let
                tokens =
                    Text.tokenize text

                removeToken token current =
                    case Text.processTerm token of
                        "" ->
                            current

                        term ->
                            removeTerm fieldId shortId term current
            in
            List.foldl removeToken (removeFieldLength fieldId miniSearch.documentCount (uniqueCount tokens) miniSearch) tokens

        Nothing ->
            miniSearch


{-| `removeTerm`: one occurrence fewer of `term` in the field of the document; the term leaves the
tree when it has no field left.
-}
removeTerm : Int -> Int -> String -> MiniSearch -> MiniSearch
removeTerm fieldId shortId term miniSearch =
    case SearchableMap.get term miniSearch.index of
        Nothing ->
            warnDocumentChanged shortId fieldId term miniSearch

        Just fields ->
            let
                postings =
                    Maybe.withDefault Dict.empty (Dict.get fieldId fields)
            in
            case Dict.get shortId postings of
                Nothing ->
                    -- The term keeps its data, so it stays.
                    warnDocumentChanged shortId fieldId term miniSearch

                Just frequency ->
                    let
                        remaining =
                            if frequency <= 1 then
                                let
                                    others =
                                        Dict.remove shortId postings
                                in
                                -- The field goes when this was its only posting.
                                if Dict.isEmpty others then
                                    Dict.remove fieldId fields

                                else
                                    Dict.insert fieldId others fields

                            else
                                Dict.insert fieldId (Dict.insert shortId (frequency - 1) postings) fields
                    in
                    { miniSearch
                        | index =
                            if Dict.isEmpty remaining then
                                SearchableMap.remove term miniSearch.index

                            else
                                SearchableMap.update term (\_ -> remaining) miniSearch.index
                    }


warnDocumentChanged : Int -> Int -> String -> MiniSearch -> MiniSearch
warnDocumentChanged shortId fieldId term miniSearch =
    case List.head (List.drop fieldId miniSearch.options.fields) of
        Just fieldName ->
            let
                id =
                    Dict.get shortId miniSearch.documentIds |> Maybe.map idToString |> Maybe.withDefault "undefined"
            in
            warn
                ("MiniSearch: document with ID "
                    ++ id
                    ++ " has changed before removal: term \""
                    ++ term
                    ++ "\" was not present in field \""
                    ++ fieldName
                    ++ "\". Removing a document after it has changed can corrupt the index!"
                )
                miniSearch

        Nothing ->
            miniSearch


{-| The argument of `removeAll`: none, or what was passed (`Nothing` for `null`).
-}
type RemoveAllArgument
    = NoArgument
    | Argument (Maybe (List Document))


{-| `removeAll(documents)` removes each document in turn; `removeAll()` starts a new, empty index,
keeping `dirtCount` and the vacuum state.
-}
removeAll : RemoveAllArgument -> MiniSearch -> ( MiniSearch, Result String () )
removeAll argument miniSearch =
    case argument of
        Argument (Just documents) ->
            each remove documents miniSearch

        Argument Nothing ->
            ( miniSearch, Err "Expected documents to be present. Omit the argument to remove all documents." )

        NoArgument ->
            ( { miniSearch
                | index = SearchableMap.empty
                , documentCount = 0
                , documentIds = Dict.empty
                , idToShortId = emptyIdTable
                , fieldLength = Dict.empty
                , avgFieldLength = Dict.empty
                , storedFields = Dict.empty
                , nextId = 0
              }
            , Ok ()
            )



-- DISCARDING AND VACUUMING


{-| `discard(id)`, then the automatic vacuum check.
-}
discard : Id -> MiniSearch -> Result String MiniSearch
discard id miniSearch =
    Result.map maybeAutoVacuum (discardDocument id miniSearch)


{-| `discard` without its automatic vacuum check, which `discardAll` turns off for its discards.
-}
discardDocument : Id -> MiniSearch -> Result String MiniSearch
discardDocument id miniSearch =
    case shortIdOf id miniSearch.idToShortId of
        Nothing ->
            Err ("MiniSearch: cannot discard document with ID " ++ idToString id ++ ": it is not in the index")

        Just shortId ->
            let
                lengths =
                    Maybe.withDefault Dict.empty (Dict.get shortId miniSearch.fieldLength)

                withoutDocument =
                    { miniSearch
                        | idToShortId = removeId id miniSearch.idToShortId
                        , documentIds = Dict.remove shortId miniSearch.documentIds
                        , storedFields = Dict.remove shortId miniSearch.storedFields
                    }

                -- `forEach` over the document's field lengths skips the holes.
                averaged =
                    Dict.foldl (\fieldId length current -> removeFieldLength fieldId miniSearch.documentCount length current) withoutDocument lengths
            in
            Ok
                { averaged
                    | fieldLength = Dict.remove shortId averaged.fieldLength
                    , documentCount = averaged.documentCount - 1
                    , dirtCount = averaged.dirtCount + 1
                }


{-| `discardAll(ids)`: each discard without automatic vacuuming, then one automatic check; an error
ends it before that check.
-}
discardAll : List Id -> MiniSearch -> ( MiniSearch, Result String () )
discardAll ids miniSearch =
    case each discardDocument ids miniSearch of
        ( discarded, Ok () ) ->
            ( maybeAutoVacuum discarded, Ok () )

        failed ->
            failed


{-| `replace(document)`: `discard(document[idField])` (which may vacuum), then `add(document)`.
-}
replace : Document -> MiniSearch -> ( MiniSearch, Result String () )
replace document miniSearch =
    let
        discarded =
            case Dict.get miniSearch.options.idField document of
                Just (Text text) ->
                    discard (StringId text) miniSearch

                Just (Number number) ->
                    discard (NumberId number) miniSearch

                Just Null ->
                    Err "MiniSearch: cannot discard document with ID null: it is not in the index"

                Nothing ->
                    Err "MiniSearch: cannot discard document with ID undefined: it is not in the index"
    in
    case discarded of
        Ok next ->
            case add document next of
                Ok added ->
                    ( added, Ok () )

                Err message ->
                    ( next, Err message )

        Err message ->
            ( miniSearch, Err message )


maybeAutoVacuum : MiniSearch -> MiniSearch
maybeAutoVacuum miniSearch =
    case miniSearch.options.autoVacuum of
        AutoVacuumWith { batchSize, minDirtCount, minDirtFactor } ->
            conditionalVacuum { batchSize = batchSize } (Just { minDirtCount = minDirtCount, minDirtFactor = minDirtFactor }) miniSearch

        AutoVacuumFlag _ ->
            -- `false` (`true` became the defaults in `new`).
            miniSearch


{-| `vacuum(options)`: a conditional vacuum without conditions.
-}
vacuum : VacuumOptions -> MiniSearch -> MiniSearch
vacuum options miniSearch =
    conditionalVacuum options Nothing miniSearch


{-| `conditionalVacuum`. A request made while a vacuum is current is chained behind it, which in a
synchronous run means never; once a vacuum has started, MiniSearch clears its current mark only in
a later microtask, so it stays current for the rest of the run (CONTRACT.md, "Vacuuming in a
synchronous run").
-}
conditionalVacuum : VacuumOptions -> Maybe VacuumConditions -> MiniSearch -> MiniSearch
conditionalVacuum options conditions miniSearch =
    if miniSearch.currentVacuum then
        miniSearch

    else if not (vacuumConditionsMet conditions miniSearch) then
        miniSearch

    else
        performVacuuming options { miniSearch | currentVacuum = True }


{-| `dirtCount >= (minDirtCount || 20) && dirtFactor >= (minDirtFactor || 0.1)`; no conditions are
always met.
-}
vacuumConditionsMet : Maybe VacuumConditions -> MiniSearch -> Bool
vacuumConditionsMet conditions miniSearch =
    case conditions of
        Nothing ->
            True

        Just { minDirtCount, minDirtFactor } ->
            (toFloat miniSearch.dirtCount >= orDefault defaultMinDirtCount minDirtCount)
                && (dirtFactor miniSearch >= orDefault defaultMinDirtFactor minDirtFactor)


{-| `value || fallback` for an optional number: absent, 0 and NaN are falsy.
-}
orDefault : Float -> Maybe Float -> Float
orDefault fallback value =
    case value of
        Just number ->
            if number == 0 || isNaN number then
                fallback

            else
                number

        Nothing ->
            fallback


{-| The synchronous part of `performVacuuming`: walk the terms in iteration order, removing the
postings of documents no longer present (a term left with no field is deleted, with the merges
that causes); after every `batchSize`-th term MiniSearch awaits a timer, after `run` has returned,
so the walk stops there. Only a walk that ends without stopping gives the dirt count back.
-}
performVacuuming : VacuumOptions -> MiniSearch -> MiniSearch
performVacuuming options miniSearch =
    let
        initialDirtCount =
            miniSearch.dirtCount

        -- `options.batchSize || 1000`
        batchSize =
            case options.batchSize of
                Just size ->
                    if size == 0 then
                        defaultBatchSize

                    else
                        size

                Nothing ->
                    defaultBatchSize

        walk i terms current =
            case terms of
                [] ->
                    { current | dirtCount = current.dirtCount - initialDirtCount }

                term :: rest ->
                    let
                        cleaned =
                            vacuumTerm term current
                    in
                    if modBy batchSize i == 0 then
                        cleaned

                    else
                        walk (i + 1) rest cleaned
    in
    walk 1 (SearchableMap.keys miniSearch.index) miniSearch


vacuumTerm : String -> MiniSearch -> MiniSearch
vacuumTerm term miniSearch =
    case SearchableMap.get term miniSearch.index of
        Just fields ->
            let
                present shortId _ =
                    Dict.member shortId miniSearch.documentIds

                cleanField fieldId postings remaining =
                    let
                        kept =
                            Dict.filter present postings
                    in
                    if Dict.isEmpty kept then
                        Dict.remove fieldId remaining

                    else
                        Dict.insert fieldId kept remaining

                cleaned =
                    Dict.foldl cleanField fields fields
            in
            { miniSearch
                | index =
                    if Dict.isEmpty cleaned then
                        SearchableMap.remove term miniSearch.index

                    else
                        SearchableMap.update term (\_ -> cleaned) miniSearch.index
            }

        Nothing ->
            miniSearch



-- SEARCHING


{-| A query: a string, `MiniSearch.wildcard`, or a combination of queries with its own options.
-}
type Query
    = Terms String
    | Wildcard
    | Combination SearchOptions (List Query)


{-| A search result, `match` in `Object.keys` order (which `terms` is), followed by the stored
fields that `Object.assign` added to it.
-}
type alias SearchResult =
    { id : Id
    , score : Float
    , terms : List String
    , queryTerms : List String
    , match : List ( String, List String )
    , storedFields : StoredFields
    }


{-| `result[name]` for an id or a stored field: `Object.assign` lets a stored field named `id`
replace the id.
-}
resultField : String -> SearchResult -> Maybe Value
resultField name result =
    case lookupStored name result.storedFields of
        Just value ->
            Just value

        Nothing ->
            if name == "id" then
                case result.id of
                    StringId text ->
                        Just (Text text)

                    NumberId number ->
                        Just (Number number)

            else
                Nothing


lookupStored : String -> StoredFields -> Maybe Value
lookupStored name stored =
    case stored of
        [] ->
            Nothing

        ( key, value ) :: rest ->
            if key == name then
                Just value

            else
                lookupStored name rest


type alias Suggestion =
    { suggestion : String, terms : List String, score : Float }


{-| A raw result of `executeQuery`: the score, the matched query terms, and `match` (derived term
to field names) in insertion order.
-}
type alias RawResult =
    { score : Float, terms : List String, match : List ( String, List String ) }


type alias RawResults =
    OrderedDict Int RawResult


{-| `search(query, searchOptions)`.
-}
search : Query -> SearchOptions -> MiniSearch -> ( MiniSearch, Result String (List SearchResult) )
search query searchOptions miniSearch =
    let
        withDefaults =
            merge miniSearch.options.searchOptions searchOptions
    in
    case executeQuery query searchOptions miniSearch of
        ( searched, Ok rawResults ) ->
            let
                results =
                    OrderedDict.foldl (collectResult searched withDefaults.filter) [] rawResults
                        |> List.reverse
            in
            -- A wildcard query without a document boost is not sorted: every score is 1.
            ( searched
            , Ok
                (case ( query, withDefaults.boostDocument ) of
                    ( Wildcard, Nothing ) ->
                        results

                    _ ->
                        List.sortWith byScore results
                )
            )

        ( searched, Err message ) ->
            ( searched, Err message )


collectResult : MiniSearch -> Maybe (SearchResult -> Bool) -> Int -> RawResult -> List SearchResult -> List SearchResult
collectResult miniSearch filter shortId raw results =
    case Dict.get shortId miniSearch.documentIds of
        Just id ->
            let
                -- The quality is the number of distinct matched query terms.
                quality =
                    case List.length raw.terms of
                        0 ->
                            1

                        count ->
                            count

                match =
                    Text.inPropertyOrder raw.match

                result =
                    { id = id
                    , score = raw.score * toFloat quality
                    , terms = List.map Tuple.first match
                    , queryTerms = raw.terms
                    , match = match
                    , storedFields = Maybe.withDefault [] (Dict.get shortId miniSearch.storedFields)
                    }
            in
            case filter of
                Just keep ->
                    if keep result then
                        result :: results

                    else
                        results

                Nothing ->
                    result :: results

        Nothing ->
            -- Not reached: results only hold present documents.
            results


{-| `byScore`: `(a, b) => b.score - a.score`, for a stable sort.
-}
byScore : { a | score : Float } -> { a | score : Float } -> Order
byScore a b =
    compare b.score a.score


{-| `autoSuggest(queryString, options)`: search with the autoSuggest defaults, then group the hits
by their matched terms.
-}
autoSuggest : String -> SearchOptions -> MiniSearch -> ( MiniSearch, Result String (List Suggestion) )
autoSuggest queryString options miniSearch =
    case search (Terms queryString) (merge miniSearch.options.autoSuggestOptions options) miniSearch of
        ( searched, Ok hits ) ->
            let
                group hit suggestions =
                    let
                        phrase =
                            String.join " " hit.terms
                    in
                    case OrderedDict.get phrase suggestions of
                        Just suggestion ->
                            OrderedDict.insert phrase { suggestion | score = suggestion.score + hit.score, count = suggestion.count + 1 } suggestions

                        Nothing ->
                            OrderedDict.insert phrase { score = hit.score, terms = hit.terms, count = 1 } suggestions

                results =
                    List.foldl group OrderedDict.empty hits
                        |> OrderedDict.toList
                        |> List.map (\( phrase, { score, terms, count } ) -> { suggestion = phrase, terms = terms, score = score / toFloat count })
            in
            ( searched, Ok (List.sortWith byScore results) )

        ( searched, Err message ) ->
            ( searched, Err message )


{-| `executeQuery(query, searchOptions)`. A combination only sees the call's options and its own:
the index defaults come in for its string subqueries.
-}
executeQuery : Query -> SearchOptions -> MiniSearch -> ( MiniSearch, Result String RawResults )
executeQuery query searchOptions miniSearch =
    case query of
        Wildcard ->
            ( miniSearch, Ok (executeWildcardQuery searchOptions miniSearch) )

        Combination nodeOptions queries ->
            let
                options =
                    merge searchOptions nodeOptions
            in
            case executeQueries queries options miniSearch [] of
                ( searched, Ok results ) ->
                    ( searched, combineResults results options.combineWith )

                ( searched, Err message ) ->
                    ( searched, Err message )

        Terms text ->
            let
                options =
                    merge miniSearch.options.searchOptions searchOptions

                terms =
                    Text.tokenize text
                        |> List.map Text.processTerm
                        |> List.filter (not << String.isEmpty)

                ( searched, results ) =
                    List.foldl
                        (\spec ( current, done ) ->
                            let
                                ( next, result ) =
                                    executeQuerySpec spec options current
                            in
                            ( next, result :: done )
                        )
                        ( miniSearch, [] )
                        (List.indexedMap (termToQuerySpec options terms) terms)
            in
            ( searched, combineResults (List.reverse results) options.combineWith )


{-| Executes subqueries in order; the first error ends it.
-}
executeQueries : List Query -> SearchOptions -> MiniSearch -> List RawResults -> ( MiniSearch, Result String (List RawResults) )
executeQueries queries options miniSearch done =
    case queries of
        [] ->
            ( miniSearch, Ok (List.reverse done) )

        subquery :: rest ->
            case executeQuery subquery options miniSearch of
                ( searched, Ok results ) ->
                    executeQueries rest options searched (results :: done)

                ( searched, Err message ) ->
                    ( searched, Err message )


executeWildcardQuery : SearchOptions -> MiniSearch -> RawResults
executeWildcardQuery searchOptions miniSearch =
    let
        options =
            merge miniSearch.options.searchOptions searchOptions

        score shortId id =
            case options.boostDocument of
                Just boostDocument ->
                    boostDocument id "" (Dict.get shortId miniSearch.storedFields)

                Nothing ->
                    1
    in
    Dict.foldl (\shortId id results -> OrderedDict.insert shortId (RawResult (score shortId id) [] []) results) OrderedDict.empty miniSearch.documentIds


{-| A query term with its options: `termToQuerySpec(options)(term, i, terms)`.
-}
type alias QuerySpec =
    { term : String, fuzzy : Fuzzy, prefix : Bool, termBoost : Float }


termToQuerySpec : SearchOptions -> List String -> Int -> String -> QuerySpec
termToQuerySpec options terms i term =
    { term = term
    , fuzzy =
        -- `options.fuzzy || false`
        case options.fuzzy of
            Just (FuzzyNumber number) ->
                if number == 0 || isNaN number then
                    FuzzyFlag False

                else
                    FuzzyNumber number

            Just flag ->
                flag

            Nothing ->
                FuzzyFlag False
    , prefix =
        case options.prefix of
            Just (PrefixWhen when) ->
                when term i terms

            Just (PrefixFlag flag) ->
                flag

            Nothing ->
                False
    , termBoost =
        case options.boostTerm of
            Just boostTerm ->
                boostTerm term i terms

            Nothing ->
                1
    }


{-| What scores the postings of one query spec.
-}
type alias Scoring =
    { fieldBoosts : List ( String, Float )
    , boostDocument : Maybe (Id -> String -> Maybe StoredFields -> Float)
    , bm25 : Bm25
    , termBoost : Float
    }


{-| `executeQuerySpec(query, searchOptions)`: the exact term, then the prefix matches, then the
fuzzy matches that are not prefix matches.
-}
executeQuerySpec : QuerySpec -> SearchOptions -> MiniSearch -> ( MiniSearch, RawResults )
executeQuerySpec spec searchOptions miniSearch =
    let
        options =
            merge miniSearch.options.searchOptions searchOptions

        weights =
            Maybe.withDefault { fuzzy = Nothing, prefix = Nothing } options.weights

        fuzzyWeight =
            Maybe.withDefault 0.45 weights.fuzzy

        prefixWeight =
            Maybe.withDefault 0.375 weights.prefix

        scoring =
            { fieldBoosts = fieldBoosts (Maybe.withDefault miniSearch.options.fields options.fields) (Maybe.withDefault Dict.empty options.boost)
            , boostDocument = options.boostDocument
            , bm25 = Maybe.withDefault defaultBm25 options.bm25
            , termBoost = spec.termBoost
            }

        termLength =
            String.length spec.term

        -- Scoring the exact term may delete it (search's cleanup of discarded postings) and merge
        -- nodes, so the prefix view and the fuzzy walk are taken from the index it leaves.
        exact =
            termResults scoring (Derivation spec.term spec.term 1) (SearchableMap.get spec.term miniSearch.index) ( miniSearch, OrderedDict.empty )

        afterExact =
            Tuple.first exact

        prefixMatches =
            if spec.prefix then
                SearchableMap.entries (SearchableMap.atPrefix spec.term afterExact.index)
                    |> List.filter (\( term, _ ) -> String.length term /= termLength)

            else
                []

        fuzzyMatches =
            case spec.fuzzy of
                FuzzyFlag False ->
                    []

                _ ->
                    let
                        fuzzy =
                            case spec.fuzzy of
                                FuzzyNumber number ->
                                    number

                                FuzzyFlag _ ->
                                    0.2

                        maxDistance =
                            if fuzzy < 1 then
                                min (Maybe.withDefault 6 options.maxFuzzy) (round (toFloat termLength * fuzzy))

                            else
                                round fuzzy
                    in
                    if maxDistance == 0 then
                        []

                    else
                        SearchableMap.fuzzyGet spec.term maxDistance afterExact.index

        prefixed =
            List.foldl
                (\( term, data ) acc ->
                    let
                        length =
                            toFloat (String.length term)

                        distance =
                            toFloat (String.length term - termLength)

                        weight =
                            prefixWeight * length / (length + 0.3 * distance)
                    in
                    termResults scoring (Derivation spec.term term weight) (Just data) acc
                )
                exact
                prefixMatches

        -- A term that is also a prefix match is scored as a prefix match only.
        prefixTerms =
            Set.fromList (List.map Tuple.first prefixMatches)
    in
    List.foldl
        (\( term, data, distance ) acc ->
            if distance == 0 || Set.member term prefixTerms then
                acc

            else
                let
                    length =
                        toFloat (String.length term)

                    weight =
                        fuzzyWeight * length / (length + toFloat distance)
                in
                termResults scoring (Derivation spec.term term weight) (Just data) acc
        )
        prefixed
        fuzzyMatches


{-| The field boosts: for each field to search, in order (a repeated field keeps its first
place), `getOwnProperty(boost, field) || 1`.
-}
fieldBoosts : List String -> Dict String Float -> List ( String, Float )
fieldBoosts fields boost =
    List.foldl
        (\field boosts ->
            if List.any (\( name, _ ) -> name == field) boosts then
                boosts

            else
                let
                    factor =
                        case Dict.get field boost of
                            Just value ->
                                if value == 0 || isNaN value then
                                    1

                                else
                                    value

                            Nothing ->
                                1
                in
                boosts ++ [ ( field, factor ) ]
        )
        []
        fields


{-| What a derived term is scored as: the query term it comes from, itself, and its weight.
-}
type alias Derivation =
    { sourceTerm : String, derivedTerm : String, termWeight : Float }


{-| `termResults(sourceTerm, derivedTerm, termWeight, termBoost, fieldTermData, ...)`: scores the
postings of the derived term, field by field, in the order of the field boosts.
-}
termResults : Scoring -> Derivation -> Maybe FieldsData -> ( MiniSearch, RawResults ) -> ( MiniSearch, RawResults )
termResults scoring derivation fieldTermData state =
    case fieldTermData of
        Nothing ->
            state

        Just fields ->
            List.foldl
                (\( field, fieldBoost ) (( miniSearch, _ ) as acc) ->
                    case Dict.get field miniSearch.fieldIds of
                        Just fieldId ->
                            case Dict.get fieldId fields of
                                Just postings ->
                                    fieldResults scoring derivation ( field, fieldId, fieldBoost ) postings acc

                                Nothing ->
                                    acc

                        Nothing ->
                            -- A field the index does not have.
                            acc
                )
                state
                scoring.fieldBoosts


{-| One field of `termResults`, its postings in order. A posting of a document that is no longer
present is removed (`removeTerm`, which may delete the term) and lowers `matchingFields` for the
postings after it; the postings before it were scored with the higher count.
-}
fieldResults : Scoring -> Derivation -> ( String, Int, Float ) -> Dict Int Int -> ( MiniSearch, RawResults ) -> ( MiniSearch, RawResults )
fieldResults scoring { sourceTerm, derivedTerm, termWeight } ( field, fieldId, fieldBoost ) postings ( miniSearch, results ) =
    let
        avgFieldLength =
            Maybe.withDefault (0 / 0) (Dict.get fieldId miniSearch.avgFieldLength)

        scorePosting shortId termFreq ( current, scored, matchingFields ) =
            case Dict.get shortId current.documentIds of
                Nothing ->
                    ( removeTerm fieldId shortId derivedTerm current, scored, matchingFields - 1 )

                Just id ->
                    let
                        docBoost =
                            case scoring.boostDocument of
                                Just boostDocument ->
                                    boostDocument id derivedTerm (Dict.get shortId current.storedFields)

                                Nothing ->
                                    1
                    in
                    if docBoost == 0 || isNaN docBoost then
                        ( current, scored, matchingFields )

                    else
                        let
                            fieldLength =
                                Dict.get shortId current.fieldLength
                                    |> Maybe.andThen (Dict.get fieldId)
                                    |> Maybe.map toFloat
                                    |> Maybe.withDefault (0 / 0)

                            rawScore =
                                calcBM25Score (toFloat termFreq) (toFloat matchingFields) (toFloat current.documentCount) fieldLength avgFieldLength scoring.bm25

                            weightedScore =
                                termWeight * scoring.termBoost * fieldBoost * docBoost * rawScore
                        in
                        ( current, addScore shortId sourceTerm derivedTerm field weightedScore scored, matchingFields )

        ( after, afterResults, _ ) =
            Dict.foldl scorePosting ( miniSearch, results, Dict.size postings ) postings
    in
    ( after, afterResults )


addScore : Int -> String -> String -> String -> Float -> RawResults -> RawResults
addScore shortId sourceTerm derivedTerm field weightedScore results =
    case OrderedDict.get shortId results of
        Just result ->
            OrderedDict.insert shortId
                (RawResult (result.score + weightedScore) (assignUniqueTerm result.terms sourceTerm) (addMatch derivedTerm field result.match))
                results

        Nothing ->
            OrderedDict.insert shortId (RawResult weightedScore [ sourceTerm ] [ ( derivedTerm, [ field ] ) ]) results


{-| `match[derivedTerm].push(field)`, or `match[derivedTerm] = [field]`.
-}
addMatch : String -> String -> List ( String, List String ) -> List ( String, List String )
addMatch term field match =
    case match of
        [] ->
            [ ( term, [ field ] ) ]

        (( key, fields ) as entry) :: rest ->
            if key == term then
                ( key, fields ++ [ field ] ) :: rest

            else
                entry :: addMatch term field rest


assignUniqueTerm : List String -> String -> List String
assignUniqueTerm terms term =
    if List.member term terms then
        terms

    else
        terms ++ [ term ]


assignUniqueTerms : List String -> List String -> List String
assignUniqueTerms target source =
    List.foldl (\term terms -> assignUniqueTerm terms term) target source


{-| `Object.assign(target, source)` of two `match` objects: an existing term keeps its place and
takes the new field list.
-}
assignMatch : List ( String, List String ) -> List ( String, List String ) -> List ( String, List String )
assignMatch target source =
    List.foldl
        (\( term, fields ) match ->
            if List.any (\( key, _ ) -> key == term) match then
                List.map
                    (\(( key, _ ) as entry) ->
                        if key == term then
                            ( key, fields )

                        else
                            entry
                    )
                    match

            else
                match ++ [ ( term, fields ) ]
        )
        target
        source


{-| BM25+, in MiniSearch's operation order. `logBase e x` is `Math.log(x) / Math.log(Math.E)`: the
host's `Math.log`, exactly, where `Math.log(Math.E)` is 1 (NOTES.md checks it in every engine).
-}
calcBM25Score : Float -> Float -> Float -> Float -> Float -> Bm25 -> Float
calcBM25Score termFreq matchingCount totalCount fieldLength avgFieldLength { k, b, d } =
    let
        invDocFreq =
            logBase e (1 + (totalCount - matchingCount + 0.5) / (matchingCount + 0.5))
    in
    invDocFreq * (d + termFreq * (k + 1) / (termFreq + k * (1 - b + b * fieldLength / avgFieldLength)))


{-| `combineResults(results, combineWith = 'or')`: nothing to combine is an empty map whatever the
operator; otherwise the operator (in any letter case) folds the results from the left.
-}
combineResults : List RawResults -> Maybe String -> Result String RawResults
combineResults results combineWith =
    case results of
        [] ->
            Ok OrderedDict.empty

        first :: rest ->
            let
                operator =
                    Maybe.withDefault operatorOr combineWith

                lower =
                    String.toLower operator
            in
            if lower == operatorOr then
                Ok (List.foldl (\b a -> combineOr a b) first rest)

            else if lower == operatorAnd then
                Ok (List.foldl (\b a -> combineAnd a b) first rest)

            else if lower == operatorAndNot then
                Ok (List.foldl (\b a -> combineAndNot a b) first rest)

            else
                Err ("Invalid combination operator: " ++ operator)


{-| OR: `b`'s documents in `b`'s order, appended to `a` or summed into `a`'s entry.
-}
combineOr : RawResults -> RawResults -> RawResults
combineOr a b =
    OrderedDict.foldl
        (\shortId result combined ->
            case OrderedDict.get shortId combined of
                Nothing ->
                    OrderedDict.insert shortId result combined

                Just existing ->
                    OrderedDict.insert shortId
                        (RawResult (existing.score + result.score) (assignUniqueTerms existing.terms result.terms) (assignMatch existing.match result.match))
                        combined
        )
        a
        b


{-| AND: a new map, in `b`'s order, of the documents in both.
-}
combineAnd : RawResults -> RawResults -> RawResults
combineAnd a b =
    OrderedDict.foldl
        (\shortId result combined ->
            case OrderedDict.get shortId a of
                Nothing ->
                    combined

                Just existing ->
                    OrderedDict.insert shortId
                        (RawResult (existing.score + result.score) (assignUniqueTerms existing.terms result.terms) (assignMatch existing.match result.match))
                        combined
        )
        OrderedDict.empty
        b


{-| AND_NOT: `a` without `b`'s documents.
-}
combineAndNot : RawResults -> RawResults -> RawResults
combineAndNot a b =
    OrderedDict.foldl (\shortId _ remaining -> OrderedDict.remove shortId remaining) a b



-- LOOKUPS AND STATE


has : Id -> MiniSearch -> Bool
has id miniSearch =
    isJust (shortIdOf id miniSearch.idToShortId)


isJust : Maybe a -> Bool
isJust maybe =
    case maybe of
        Just _ ->
            True

        Nothing ->
            False


{-| `getStoredFields(id)`: `Nothing` for an absent document, and for every document when there are
no `storeFields`.
-}
getStoredFields : Id -> MiniSearch -> Maybe StoredFields
getStoredFields id miniSearch =
    shortIdOf id miniSearch.idToShortId |> Maybe.andThen (\shortId -> Dict.get shortId miniSearch.storedFields)


documentCount : MiniSearch -> Int
documentCount miniSearch =
    miniSearch.documentCount


termCount : MiniSearch -> Int
termCount miniSearch =
    SearchableMap.size miniSearch.index


dirtCount : MiniSearch -> Int
dirtCount miniSearch =
    miniSearch.dirtCount


dirtFactor : MiniSearch -> Float
dirtFactor miniSearch =
    toFloat miniSearch.dirtCount / toFloat (1 + miniSearch.documentCount + miniSearch.dirtCount)


isVacuuming : MiniSearch -> Bool
isVacuuming miniSearch =
    miniSearch.currentVacuum


{-| `toJSON()`. Arrays with holes are lists of `Maybe`; integer-keyed objects are lists in
ascending key order.
-}
type alias Serialized =
    { documentCount : Int
    , nextId : Int
    , documentIds : List ( Int, Id )
    , fieldIds : List ( String, Int )
    , fieldLength : List ( Int, List (Maybe Int) )
    , averageFieldLength : List (Maybe Float)
    , storedFields : List ( Int, StoredFields )
    , dirtCount : Int
    , index : List ( String, List ( Int, List ( Int, Int ) ) )
    , serializationVersion : Int
    }


toJson : MiniSearch -> Serialized
toJson miniSearch =
    { documentCount = miniSearch.documentCount
    , nextId = miniSearch.nextId
    , documentIds = Dict.toList miniSearch.documentIds
    , fieldIds = List.indexedMap (\i field -> ( field, i )) miniSearch.options.fields
    , fieldLength = Dict.toList (Dict.map (\_ lengths -> withHoles lengths) miniSearch.fieldLength)
    , averageFieldLength = withHoles miniSearch.avgFieldLength
    , storedFields = Dict.toList miniSearch.storedFields
    , dirtCount = miniSearch.dirtCount
    , index =
        SearchableMap.entries miniSearch.index
            |> List.map (\( term, fields ) -> ( term, Dict.toList (Dict.map (\_ postings -> Dict.toList postings) fields) ))
    , serializationVersion = 2
    }


{-| An array whose length is one past its highest index, holes where nothing was set.
-}
withHoles : Dict Int a -> List (Maybe a)
withHoles values =
    case Dict.keys values |> List.maximum of
        Just highest ->
            List.map (\i -> Dict.get i values) (List.range 0 highest)

        Nothing ->
            []
