module Values exposing (V(..), array, asNumber, asString, decode, encode, equal, field, fields, fromJson, index, kind, jsonText, put, raw, sortFields, stringValue, tag, toNumber, truth, wire)

import Array exposing (Array)
import Dict
import Json.Decode as D
import Json.Encode as E


type V
    = Missing
    | Null
    | Flag Bool
    | Num Float
    | Str String
    | Arr (List V)
    | Obj (List ( String, V ))
    | Box E.Value V


raw v =
    case v of
        Box _ plain ->
            plain

        _ ->
            v



-- The host compares these opaque identity tokens with === for contains().
-- Constructing a new array or object allocates a fresh token; indexing or
-- projecting a previously constructed object preserves its existing one.


tag v =
    case v of
        Arr xs ->
            Box (E.list E.int [ List.length xs ]) v

        Obj xs ->
            Box (E.list E.int [ List.length xs ]) v

        _ ->
            v


array v =
    case raw v of
        Arr a ->
            a

        _ ->
            []


fields v =
    case raw v of
        Obj o ->
            o

        Arr a ->
            List.indexedMap (\i x -> ( String.fromInt i, x )) a

        Str s ->
            List.range 0 (String.length s - 1) |> List.map (\i -> ( String.fromInt i, Str (String.slice i (i + 1) s) ))

        _ ->
            []


asString v =
    case v of
        Str s ->
            s

        _ ->
            ""


asNumber v =
    case v of
        Num x ->
            x

        _ ->
            0


index i xs =
    if i < 0 then
        Missing

    else
        Maybe.withDefault Missing (List.head (List.drop i xs))


field key v =
    fields v |> List.filter (Tuple.first >> (==) key) |> List.head |> Maybe.map Tuple.second |> Maybe.withDefault Missing


-- Hash and merge results use ordinary JS assignment. Input objects are
-- decoded separately and retain every own data property.
put key v xs =
    if key == "__proto__" then
        xs

    else if List.any (Tuple.first >> (==) key) xs then
        List.map
            (\( k, x ) ->
                ( k
                , if k == key then
                    v

                  else
                    x
                )
            )
            xs

    else
        xs ++ [ ( key, v ) ]


kind v =
    case v of
        Box _ plain ->
            kind plain

        Missing ->
            "undefined"

        Null ->
            "null"

        Flag _ ->
            "boolean"

        Num _ ->
            "number"

        Str _ ->
            "string"

        Arr _ ->
            "array"

        Obj _ ->
            if field "jmespathType" v == Str "Expref" then
                "expref"

            else
                "object"



-- JavaScript enumerates canonical array-index keys first, then other keys in insertion order.


sortFields xs =
    let
        intKey s =
            case String.toInt s of
                Just n ->
                    if n >= 0 && n < 4294967295 && String.fromInt n == s then
                        Just n

                    else
                        Nothing

                Nothing ->
                    Nothing

        order ( a, _ ) ( b, _ ) =
            case ( intKey a, intKey b ) of
                ( Just i, Just j ) ->
                    compare i j

                ( Just _, Nothing ) ->
                    LT

                ( Nothing, Just _ ) ->
                    GT

                _ ->
                    EQ
    in
    List.sortWith order xs


encode v =
    case v of
        Box _ plain ->
            encode plain

        Missing ->
            E.null

        Null ->
            E.null

        Flag b ->
            E.bool b

        Num x ->
            E.float x

        Str s ->
            E.string s

        Arr a ->
            E.list encode a

        Obj o ->
            E.object
                (List.filterMap
                    (\( k, x ) ->
                        if x == Missing then
                            Nothing

                        else
                            Just ( k, encode x )
                    )
                    o
                )


decode : D.Decoder V
decode =
    D.oneOf
        [ D.map Str D.string
        , D.map Num D.float
        , D.map Flag D.bool
        , D.null Null
        , D.map2 (\token xs -> Box token (Arr xs)) D.value (D.list (D.lazy (\_ -> decode)))
        , D.map2 (\token xs -> Box token (Obj xs)) D.value (D.keyValuePairs (D.lazy (\_ -> decode)))
        ]


fromJson s =
    D.decodeString decode s |> Result.mapError (always "syntax")


numberText x =
    if isNaN x then
        "NaN"

    else if isInfinite x then
        if x < 0 then
            "-Infinity"

        else
            "Infinity"

    else
        E.encode 0 (E.float x)


stringValue v =
    case v of
        Box _ plain ->
            stringValue plain

        Str s ->
            s

        Num x ->
            numberText x

        Flag b ->
            if b then
                "true"

            else
                "false"

        Null ->
            "null"

        Missing ->
            "undefined"

        Obj _ ->
            "[object Object]"

        Arr xs ->
            xs
                |> List.map
                    (\x ->
                        if x == Null || x == Missing then
                            ""

                        else
                            stringValue x
                    )
                |> String.join ","


toNumber v =
    case v of
        Num x ->
            x

        Null ->
            0

        Flag b ->
            if b then
                1

            else
                0

        _ ->
            let
                s =
                    String.trim (stringValue v)
            in
            if s == "" then
                0

            else
                let
                    prefix =
                        String.toLower (String.left 2 s)

                    base =
                        if prefix == "0x" then
                            16

                        else if prefix == "0o" then
                            8

                        else if prefix == "0b" then
                            2

                        else
                            0

                    digit c acc =
                        let
                            code =
                                Char.toCode (Char.toLower c)

                            d =
                                if code >= 48 && code <= 57 then
                                    code - 48

                                else if code >= 97 && code <= 102 then
                                    code - 87

                                else
                                    16
                        in
                        if d >= base then
                            0 / 0

                        else
                            acc * toFloat base + toFloat d
                in
                if base /= 0 && String.length s > 2 then
                    String.foldl digit 0 (String.dropLeft 2 s)

                else
                    String.toFloat s |> Maybe.withDefault (0 / 0)


truth v =
    case raw v of
        Null ->
            False

        Flag b ->
            b

        Str s ->
            s /= ""

        Arr xs ->
            not (List.isEmpty xs)

        Obj xs ->
            not (List.isEmpty xs)

        _ ->
            True


equal a b =
    case ( raw a, raw b ) of
        ( Obj x, Obj y ) ->
            List.length x == List.length y && List.all (\( k, v ) -> equal v (field k b)) x

        ( Arr x, Arr y ) ->
            List.length x == List.length y && List.all identity (List.map2 equal x y)

        _ ->
            raw a == raw b



-- Port values use entry lists for objects so every JSON key is ordinary data.
wire v =
    case raw v of
        Missing ->
            E.object [ ( "missing", E.bool True ) ]

        Obj xs ->
            E.object [ ( "entries", E.list (\( k, x ) -> E.list identity [ E.string k, wire x ]) xs ) ]

        Arr xs ->
            E.list wire xs

        x ->
            encode x


jsonText v =
    case raw v of
        Obj xs ->
            let
                entry ( k, x ) =
                    if x == Missing then
                        Nothing

                    else
                        Just (E.encode 0 (E.string k) ++ ":" ++ jsonText x)
            in
            "{" ++ String.join "," (List.filterMap entry xs) ++ "}"

        Arr xs ->
            "[" ++ String.join "," (List.map jsonText xs) ++ "]"

        x ->
            E.encode 0 (encode x)
