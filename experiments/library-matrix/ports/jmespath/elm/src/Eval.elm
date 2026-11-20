module Eval exposing (Step(..), bind, evaluate, map)

import Char
import Json.Encode as E
import String
import Values exposing (V(..), array, asNumber, asString, equal, field, fields, index, kind, put, raw, sortFields, stringValue, tag, toNumber, truth)



-- ECMAScript uses the active locale for min/max of strings. Elm suspends only
-- that primitive and resumes from the exact comparison in the current query.


type Step a
    = Done a
    | Error String
    | Collate String String (Order -> Step a)
    | RefEq E.Value E.Value (Bool -> Step a)


bind f result =
    case result of
        Done v ->
            f v

        Error e ->
            Error e

        Collate a b k ->
            Collate a b (k >> bind f)

        RefEq a b k ->
            RefEq a b (k >> bind f)


map f =
    bind (f >> Done)


traverse f xs =
    let
        go tail acc =
            case tail of
                [] ->
                    Done (List.reverse acc)

                x :: rest ->
                    f x |> bind (\v -> go rest (v :: acc))
    in
    go xs []


children n =
    array (field "children" n)


child i n =
    index i (children n)


compareJS a b =
    let
        primitive v =
            case raw v of
                Obj _ ->
                    Str (stringValue v)

                Arr _ ->
                    Str (stringValue v)

                _ ->
                    v

        x =
            primitive a

        y =
            primitive b
    in
    case ( x, y ) of
        ( Str i, Str j ) ->
            Just (compare i j)

        _ ->
            let
                u =
                    toNumber x

                v =
                    toNumber y
            in
            if isNaN u || isNaN v then
                Nothing

            else
                Just (compare u v)


clamped v fallback step len =
    if v == Null then
        fallback

    else
        let
            n =
                toNumber v

            i =
                if n < 0 then
                    len + n

                else
                    n
        in
        if step < 0 then
            clamp -1 (len - 1) i

        else
            clamp 0 len i


slice n xs =
    let
        len =
            toFloat (List.length xs)

        stride =
            child 2 n

        step =
            if stride == Null then
                1

            else
                toNumber stride

        start =
            clamped (child 0 n)
                (if step < 0 then
                    len - 1

                 else
                    0
                )
                step
                len

        stop =
            clamped (child 1 n)
                (if step < 0 then
                    -1

                 else
                    len
                )
                step
                len

        loop i acc =
            if (step > 0 && i < stop) || (step < 0 && i > stop) then
                loop (i + step) (index (truncate i) xs :: acc)

            else
                List.reverse acc
    in
    if step == 0 then
        Error "invalid-value"

    else
        Done (Arr (loop start []))


evaluate n v =
    evaluateRaw n v |> map tag


evaluateRaw n v =
    let
        k =
            asString (field "type" n)

        c i =
            child i n

        eval i =
            evaluate (c i) v

        next () =
            eval 0 |> bind (evaluate (c 1))

        project base filter =
            case raw base of
                Arr xs ->
                    let
                        visit item =
                            (if filter then
                                evaluate (c 2) item |> map truth

                             else
                                Done True
                            )
                                |> bind
                                    (\yes ->
                                        if yes then
                                            evaluate (c 1) item

                                        else
                                            Done Null
                                    )
                    in
                    traverse visit xs |> map (List.filter ((/=) Null) >> Arr)

                _ ->
                    Done Null
    in
    case k of
        "Field" ->
            case raw v of
                Obj _ ->
                    let
                        x =
                            field (asString (field "name" n)) v
                    in
                    Done
                        (if x == Missing then
                            Null

                         else
                            x
                        )

                _ ->
                    Done Null

        "Identity" ->
            Done v

        "Current" ->
            Done v

        "Literal" ->
            Done (field "value" n)

        "Pipe" ->
            next ()

        "IndexExpression" ->
            next ()

        "Subexpression" ->
            next ()

        "Index" ->
            case raw v of
                Arr xs ->
                    let
                        ix =
                            asNumber (field "value" n)

                        len =
                            toFloat (List.length xs)

                        pos =
                            if ix < 0 then len + ix else ix
                    in
                    if pos >= 0 && pos < len then
                        Done (index (truncate pos) xs)

                    else
                        Done Null

                _ ->
                    Done Null

        "Slice" ->
            case raw v of
                Arr xs ->
                    slice n xs

                _ ->
                    Done Null

        "Flatten" ->
            eval 0
                |> map
                    (\base ->
                        case raw base of
                            Arr xs ->
                                Arr
                                    (List.concatMap
                                        (\x ->
                                            case raw x of
                                                Arr ls ->
                                                    ls

                                                _ ->
                                                    [ x ]
                                        )
                                        xs
                                    )

                            _ ->
                                Null
                    )

        "FilterProjection" ->
            eval 0 |> bind (\base -> project base True)

        "Projection" ->
            eval 0 |> bind (\base -> project base False)

        "ValueProjection" ->
            eval 0
                |> bind
                    (\base ->
                        case raw base of
                            Obj xs ->
                                project (Arr (List.map Tuple.second xs)) False

                            _ ->
                                Done Null
                    )

        "Comparator" ->
            let
                check a b =
                    case asString (field "name" n) of
                        "EQ" ->
                            equal a b

                        "NE" ->
                            not (equal a b)

                        op ->
                            case compareJS a b of
                                Just LT ->
                                    op == "LT" || op == "LTE"

                                Just GT ->
                                    op == "GT" || op == "GTE"

                                Just EQ ->
                                    op == "LTE" || op == "GTE"

                                Nothing ->
                                    False
            in
            eval 0 |> bind (\a -> eval 1 |> map (check a >> Flag))

        "NotExpression" ->
            eval 0 |> map (truth >> not >> Flag)

        "AndExpression" ->
            eval 0
                |> bind
                    (\a ->
                        if truth a then
                            eval 1

                        else
                            Done a
                    )

        "OrExpression" ->
            eval 0
                |> bind
                    (\a ->
                        if truth a then
                            Done a

                        else
                            eval 1
                    )

        "ExpressionReference" ->
            Done (Obj (put "jmespathType" (Str "Expref") (fields (c 0))))

        "Function" ->
            traverse (\item -> evaluate item v) (children n) |> bind (call (asString (field "name" n)))

        "MultiSelectList" ->
            if v == Null then
                Done Null

            else
                traverse (\item -> evaluate item v) (children n) |> map Arr

        "MultiSelectHash" ->
            let
                pair item =
                    evaluate (field "value" item) v |> map (Tuple.pair (asString (field "name" item)))
            in
            if v == Null then
                Done Null

            else
                traverse pair (children n) |> map (List.foldl (\( name, x ) -> put name x) [] >> sortFields >> Obj)

        _ ->
            Error "syntax"


signature name =
    if List.member name [ "abs", "ceil", "floor" ] then
        [ [ "number" ] ]

    else if List.member name [ "avg", "sum" ] then
        [ [ "numbers" ] ]

    else if name == "contains" then
        [ [ "string", "array" ], [ "any" ] ]

    else if name == "starts_with" || name == "ends_with" then
        [ [ "string" ], [ "string" ] ]

    else if name == "length" then
        [ [ "object", "string", "array" ] ]

    else if name == "map" then
        [ [ "expref" ], [ "array" ] ]

    else if List.member name [ "max", "min", "sort" ] then
        [ [ "numbers", "strings" ] ]

    else if List.member name [ "keys", "values", "merge" ] then
        [ [ "object" ] ]

    else if List.member name [ "max_by", "min_by", "sort_by" ] then
        [ [ "array" ], [ "expref" ] ]

    else if name == "join" then
        [ [ "string" ], [ "strings" ] ]

    else if name == "reverse" then
        [ [ "string", "array" ] ]

    else if List.member name [ "to_array", "to_number", "to_string", "type", "not_null" ] then
        [ [ "any" ] ]

    else
        []


valid x target =
    case target of
        "any" ->
            True

        "numbers" ->
            kind x == "array" && List.all (kind >> (==) "number") (array x)

        "strings" ->
            kind x == "array" && List.all (kind >> (==) "string") (array x)

        _ ->
            kind x == target


minMax lower xs =
    let
        loop best rest =
            case rest of
                [] ->
                    Done best

                v :: tail ->
                    let
                        take ord =
                            if
                                ord
                                    == (if lower then
                                            LT

                                        else
                                            GT
                                       )
                            then
                                loop v tail

                            else
                                loop best tail
                    in
                    case ( v, best ) of
                        ( Str a, Str b ) ->
                            Collate a b take

                        _ ->
                            take (Maybe.withDefault EQ (compareJS v best))
    in
    case xs of
        [] ->
            Done Null

        a :: tail ->
            loop a tail


byKey op expr xs =
    let
        keyed v =
            evaluate expr v |> map (\key -> ( key, v ))

        check pairs =
            let
                firstType =
                    List.head pairs |> Maybe.map (Tuple.first >> kind)

                good ( k, _ ) =
                    kind k == "number" || kind k == "string"

                sorted =
                    List.sortWith (\( a, _ ) ( b, _ ) -> Maybe.withDefault EQ (compareJS a b)) pairs

                extreme ( k, v ) ( best, old ) =
                    if
                        compareJS k best
                            == Just
                                (if op == "min_by" then
                                    LT

                                 else
                                    GT
                                )
                    then
                        ( k, v )

                    else
                        ( best, old )
            in
            if not (List.all good pairs) then
                Error "invalid-type"

            else if op == "sort_by" then
                if List.all (\( key, _ ) -> Just (kind key) == firstType) pairs then
                    Done (Arr (List.map Tuple.second sorted))

                else
                    Error "invalid-type"

            else
                List.foldl extreme
                    ( Num
                        (if op == "min_by" then
                            1 / 0

                         else
                            -1 / 0
                        )
                    , Missing
                    )
                    pairs
                    |> Tuple.second
                    |> Done
    in
    traverse keyed xs |> bind check


call name args =
    let
        sig =
            signature name

        a =
            index 0 args

        b =
            index 1 args

        xs =
            array a

        text =
            asString a

        revUnits s =
            List.range 0 (String.length s - 1) |> List.map (\i -> String.slice i (i + 1) s) |> List.reverse |> String.concat

        keyOrder x y =
            compare (stringValue x) (stringValue y)

        contains target remaining =
            case remaining of
                [] ->
                    Done (Flag False)

                candidate :: tail ->
                    case ( target, candidate ) of
                        ( Box left _, Box right _ ) ->
                            RefEq left
                                right
                                (\same ->
                                    if same then
                                        Done (Flag True)

                                    else
                                        contains target tail
                                )

                        _ ->
                            if target == candidate then
                                Done (Flag True)

                            else
                                contains target tail

        merge v pairs =
            List.foldl (\( k, x ) -> put k x) pairs (fields v)

        total =
            List.foldl (\v sum -> sum + asNumber v) 0 xs
    in
    if List.isEmpty sig then
        Error "unknown-function"

    else if List.length args < List.length sig || (not (List.member name [ "merge", "not_null" ]) && List.length args > List.length sig) then
        Error "invalid-arity"

    else if not (List.all identity (List.map2 (\v types -> List.any (valid v) types) args sig)) then
        Error "invalid-type"

    else
        case name of
            "abs" ->
                Done (Num (abs (asNumber a)))

            "floor" ->
                Done (Num (toFloat (floor (asNumber a))))

            "ceil" ->
                Done (Num (toFloat (ceiling (asNumber a))))

            "type" ->
                Done (Str (kind a))

            "length" ->
                Done
                    (Num
                        (toFloat
                            (if kind a == "string" then
                                String.length text

                             else
                                List.length (fields a)
                            )
                        )
                    )

            "keys" ->
                Done (Arr (List.map (Tuple.first >> Str) (fields a)))

            "values" ->
                Done (Arr (List.map Tuple.second (fields a)))

            "contains" ->
                if kind a == "string" then
                    Done (Flag (String.contains (stringValue b) text))

                else
                    contains b xs

            "starts_with" ->
                Done
                    (Flag
                        (if asString b == "" then
                            text == ""

                         else
                            String.startsWith (asString b) text && not (String.contains (asString b) (String.dropLeft 1 text))
                        )
                    )

            "ends_with" ->
                Done (Flag (String.endsWith (asString b) text))

            "join" ->
                Done (Str (String.join text (List.map asString (array b))))

            "reverse" ->
                Done
                    (if kind a == "string" then
                        Str (revUnits text)

                     else
                        Arr (List.reverse xs)
                    )

            "sort" ->
                Done (Arr (List.sortWith keyOrder xs))

            "sum" ->
                Done (Num total)

            "avg" ->
                Done (Num (total / toFloat (List.length xs)))

            "not_null" ->
                Done (Maybe.withDefault Null (List.head (List.filter ((/=) Null) args)))

            "to_number" ->
                if kind a == "number" || (kind a == "string" && not (isNaN (toNumber a))) then
                    Done (Num (toNumber a))

                else
                    Done Null

            "to_string" ->
                Done
                    (if kind a == "string" then
                        a

                     else if a == Missing then
                        Missing

                     else
                        Str (Values.jsonText a)
                    )

            "to_array" ->
                Done
                    (if kind a == "array" then
                        a

                     else
                        Arr [ a ]
                    )

            "merge" ->
                Done (Obj (sortFields (List.foldl merge [] args)))

            "map" ->
                traverse (evaluate a) (array b) |> map Arr

            "min" ->
                minMax True xs

            "max" ->
                minMax False xs

            _ ->
                byKey name b xs
