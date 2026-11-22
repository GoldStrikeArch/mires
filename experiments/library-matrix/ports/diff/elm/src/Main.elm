port module Main exposing (main)

import Array
import Differ exposing (Kind(..))
import Json.Decode as D
import Json.Encode as E
import Patch exposing (field, string)


port request : (D.Value -> msg) -> Sub msg


port response : E.Value -> Cmd msg


main : Program () () D.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update =
            \envelope _ ->
                let
                    res =
                        case D.decodeValue (D.map2 Tuple.pair (D.field "input" D.value) (D.field "undefined" D.value)) envelope of
                            Ok ( input, absent ) ->
                                run absent input

                            Err _ ->
                                Err "missing diff request"
                in
                ( ()
                , response
                    (case res of
                        Ok v ->
                            E.object [ ( "ok", v ) ]

                        Err message ->
                            E.object [ ( "error", E.string message ) ]
                    )
                )
        , subscriptions = \_ -> request identity
        }


array v =
    D.decodeValue (D.list D.value) v |> Result.withDefault []


change kind count v =
    E.object [ ( "count", E.int count ), ( "added", E.bool (kind == Add) ), ( "removed", E.bool (kind == Remove) ), ( "value", v ) ]


{-| An array or object of a JSON document while `canonical` visits it: the key it has in its
parent, its members still to visit (array members under the key ""), and its finished members,
latest first.
-}
type alias Container =
    { key : String, isArray : Bool, pending : List ( String, D.Value ), done : List ( String, E.Value ) }


{-| The container a JSON value opens, if it is an array or an object. canonicalize calls an
object's truthy `toJSON` member, which JSON cannot make callable, so that fails as npm throws.
-}
container : String -> D.Value -> Result String (Maybe Container)
container key v =
    case D.decodeValue (D.list D.value) v of
        Ok items ->
            Ok (Just (Container key True (List.map (Tuple.pair "") items) []))

        Err _ ->
            case D.decodeValue (D.keyValuePairs D.value) v of
                Ok fields ->
                    if Patch.truthyField "toJSON" v then
                        Err "obj.toJSON is not a function"

                    else
                        Ok (Just (Container key False (List.sortBy Tuple.first fields) []))

                Err _ ->
                    Ok Nothing


{-| jsdiff's canonicalize for JSON documents: arrays in order, object keys sorted and assigned to
a plain object (so a `__proto__` key sets the prototype and is not serialized). npm recurses
once per nesting level; this visits the document with a stack of open containers in a loop, so
nesting costs heap, not call stack.
-}
canonical : D.Value -> Result String E.Value
canonical root =
    case container "" root of
        Ok (Just open) ->
            canonicalize [ open ]

        Ok Nothing ->
            Ok root

        Err e ->
            Err e


canonicalize : List Container -> Result String E.Value
canonicalize stack =
    case stack of
        [] ->
            Ok E.null

        open :: below ->
            case open.pending of
                ( key, member ) :: rest ->
                    let
                        visited =
                            { open | pending = rest }
                    in
                    case container key member of
                        Ok (Just inner) ->
                            canonicalize (inner :: visited :: below)

                        Ok Nothing ->
                            canonicalize ({ visited | done = ( key, member ) :: visited.done } :: below)

                        Err e ->
                            Err e

                [] ->
                    let
                        value =
                            if open.isArray then
                                E.list Tuple.second (List.reverse open.done)

                            else
                                E.object (List.reverse open.done)
                    in
                    case below of
                        [] ->
                            Ok value

                        parent :: rest ->
                            canonicalize ({ parent | done = ( open.key, value ) :: parent.done } :: rest)


run absent input =
    let
        get k v =
            field absent k v

        op =
            string (get "op" input)

        a =
            get "before" input

        b =
            get "after" input

        arg =
            get "patch" input

        opts =
            get "options" input

        flag k =
            D.decodeValue D.bool (get k opts) |> Result.withDefault False

        o =
            Differ.Options (flag "ignoreCase") (flag "oneChangePerToken") (flag "stripTrailingCr") (flag "ignoreWhitespace") (flag "newlineIsToken") (flag "ignoreNewlineAtEof")

        tokens v =
            Differ.tokenize op o (string v)

        -- json.js castInput: a string as it is, any other document pretty-printed canonically.
        jsonTokens v =
            case D.decodeValue D.string v of
                Ok text ->
                    Ok (Differ.tokenize op o text)

                Err _ ->
                    canonical v |> Result.map (E.encode 2 >> Differ.tokenize op o)

        output r =
            change r.kind r.count (E.string r.value)

        patch () =
            Patch.create absent input (D.decodeValue D.int (get "context" opts) |> Result.withDefault 4)

        readParsed ps =
            case ps of
                [ p ] ->
                    Patch.apply (string a) p |> Maybe.map E.string |> Maybe.withDefault (E.bool False) |> Ok

                _ ->
                    Err "multiple file patches"

        reverse =
            Patch.reverse absent
    in
    case op of
        "diffChars" ->
            Ok (E.list output (Differ.changes op o (tokens a) (tokens b)))

        "diffWords" ->
            Ok (E.list output (Differ.changes op o (tokens a) (tokens b)))

        "diffLines" ->
            Ok (E.list output (Differ.changes op o (tokens a) (tokens b)))

        "diffJson" ->
            Result.map2 (\old new -> E.list output (Differ.changes op o old new)) (jsonTokens a) (jsonTokens b)

        "diffArrays" ->
            let
                old =
                    Array.fromList (array a)

                new =
                    Array.fromList (array b)

                rows =
                    Differ.myers (Array.map key old) (Array.map key new) o.individual

                -- Elements compare by their JSON text, which is === for JSON scalars. With
                -- ignoreCase, elements are strings (CONTRACT.md) and compare by String.toLower.
                key v =
                    if o.ignoreCase then
                        D.decodeValue D.string v |> Result.map String.toLower |> Result.withDefault (E.encode 0 v)

                    else
                        E.encode 0 v

                build spans i j rev =
                    case spans of
                        [] ->
                            List.reverse rev

                        r :: rest ->
                            let
                                value =
                                    if r.kind == Remove then
                                        Array.slice i (i + r.count) old

                                    else
                                        Array.slice j (j + r.count) new

                                v =
                                    change r.kind r.count (E.array identity value)
                            in
                            build rest
                                (i
                                    + (if r.kind == Add then
                                        0

                                       else
                                        r.count
                                      )
                                )
                                (j
                                    + (if r.kind == Remove then
                                        0

                                       else
                                        r.count
                                      )
                                )
                                (v :: rev)
            in
            Ok (E.list identity (build rows 0 0 []))

        "createTwoFilesPatch" ->
            Ok (E.string (Patch.format (patch ())))

        "structuredPatch" ->
            Ok (Patch.write (patch ()))

        "parsePatch" ->
            Patch.parse (string arg) |> Result.map (E.list Patch.write)

        "applyPatch" ->
            case D.decodeValue D.string arg of
                Ok s ->
                    Patch.parse s |> Result.andThen readParsed

                Err _ ->
                    case D.decodeValue (D.list D.value) arg of
                        Ok xs ->
                            readParsed (List.map Patch.read xs)

                        Err _ ->
                            readParsed [ Patch.read arg ]

        "reversePatch" ->
            case D.decodeValue (D.list D.value) arg of
                Ok xs ->
                    Ok (E.list reverse (List.reverse xs))

                Err _ ->
                    Ok (reverse arg)

        _ ->
            Err "unknown diff operation"
