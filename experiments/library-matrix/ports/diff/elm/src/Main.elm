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
            \input _ ->
                ( ()
                , response
                    (case run input of
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


canonical v =
    case D.decodeValue (D.list D.value) v of
        Ok xs ->
            E.list canonical xs

        Err _ ->
            case D.decodeValue (D.keyValuePairs D.value) v of
                Ok xs ->
                    E.object (List.sortBy Tuple.first xs |> List.map (Tuple.mapSecond canonical))

                Err _ ->
                    v


run input =
    let
        op =
            string (field "op" input)

        a =
            field "before" input

        b =
            field "after" input

        arg =
            field "patch" input

        opts =
            field "options" input

        flag k =
            D.decodeValue D.bool (field k opts) |> Result.withDefault False

        o =
            Differ.Options (flag "ignoreCase") (flag "oneChangePerToken") (flag "stripTrailingCr") (flag "ignoreWhitespace") (flag "newlineIsToken") (flag "ignoreNewlineAtEof")

        doc v =
            if op == "diffJson" then
                D.decodeValue D.string v |> Result.withDefault (E.encode 2 (canonical v))

            else
                string v

        tokens v =
            Differ.tokenize op o (doc v)

        output r =
            change r.kind r.count (E.string r.value)

        patch () =
            Patch.create input (D.decodeValue D.int (field "context" opts) |> Result.withDefault 4)

        readParsed ps =
            case ps of
                [ p ] ->
                    Patch.apply (string a) p |> Maybe.map E.string |> Maybe.withDefault (E.bool False) |> Ok

                _ ->
                    Err "multiple file patches"

        reverse v =
            Patch.read v |> Patch.reverse |> Patch.write True
    in
    case op of
        "diffChars" ->
            Ok (E.list output (Differ.changes op o (tokens a) (tokens b)))

        "diffWords" ->
            Ok (E.list output (Differ.changes op o (tokens a) (tokens b)))

        "diffLines" ->
            Ok (E.list output (Differ.changes op o (tokens a) (tokens b)))

        "diffJson" ->
            Ok (E.list output (Differ.changes op o (tokens a) (tokens b)))

        "diffArrays" ->
            let
                old =
                    Array.fromList (array a)

                new =
                    Array.fromList (array b)

                rows =
                    Differ.myers (Array.map (E.encode 0) old) (Array.map (E.encode 0) new) o.individual

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
            Ok (Patch.write False (patch ()))

        "parsePatch" ->
            Patch.parse (string arg) |> Result.map (E.list (Patch.write False))

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
