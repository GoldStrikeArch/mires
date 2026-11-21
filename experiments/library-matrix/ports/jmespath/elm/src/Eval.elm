module Eval exposing (Step(..), andThen, evaluate)

{-| jmespath's TreeInterpreter and Runtime over `V`.

Two JS operations have no Elm counterpart, so an evaluation can stop and ask the host: `Collate`
for `localeCompare` (max and min of strings), `Identical` for `===` between tokens (contains). The
host answers and the evaluation resumes from the continuation. Between those, everything is pure.

Element loops (`evalChildren`, `evalHashPairs`, `evalMap`, `filterPass`, `projectHelp`, `evalKeys`,
`evalSortKeys`) are tail-recursive while each step is done, so wide arrays cost no stack, and
`evaluate` pattern-matches `Done` directly on the fast path so deep expressions use minimal stack.

-}

import Array
import Values exposing (Token, V(..), field, fieldList, fresh, isFalse, jsNumber, jsString, jsonText, put, relation, sortFields, strictDeepEqual)


type Step a
    = Done a
    | Failed String
    | Collate String String (Order -> Step a)
    | Identical Token (List Token) (Bool -> Step a)


andThen : (a -> Step b) -> Step a -> Step b
andThen f step =
    case step of
        Done a ->
            f a

        Failed category ->
            Failed category

        Collate a b resume ->
            Collate a b (resume >> andThen f)

        Identical target candidates resume ->
            Identical target candidates (resume >> andThen f)


map : (a -> b) -> Step a -> Step b
map f =
    andThen (f >> Done)


{-| Evaluate a list of AST child nodes against `value` in order, stopping at the first failure.
-}
evalChildren : List V -> V -> List V -> Step (List V)
evalChildren nodes value acc =
    case nodes of
        [] ->
            Done (List.reverse acc)

        c :: rest ->
            case evaluate c value of
                Done y ->
                    evalChildren rest value (y :: acc)

                Failed category ->
                    Failed category

                suspended ->
                    suspended |> andThen (\y -> evalChildren rest value (y :: acc))


{-| Evaluate `MultiSelectHash` key/value pair nodes against `value` in order.
-}
evalHashPairs : List V -> V -> List ( String, V ) -> Step (List ( String, V ))
evalHashPairs pairs value acc =
    case pairs of
        [] ->
            Done (List.reverse acc)

        pair :: rest ->
            let
                pairFields =
                    fieldList pair

                name =
                    jsString (field "name" pairFields)
            in
            case evaluate (field "value" pairFields) value of
                Done v ->
                    evalHashPairs rest value (( name, v ) :: acc)

                Failed category ->
                    Failed category

                suspended ->
                    suspended |> andThen (\v -> evalHashPairs rest value (( name, v ) :: acc))


{-| Evaluate `expr` over each element of `items` in order, stopping at the first failure.
-}
evalMap : V -> List V -> List V -> Step (List V)
evalMap expr items acc =
    case items of
        [] ->
            Done (List.reverse acc)

        x :: rest ->
            case evaluate expr x of
                Done y ->
                    evalMap expr rest (y :: acc)

                Failed category ->
                    Failed category

                suspended ->
                    suspended |> andThen (\y -> evalMap expr rest (y :: acc))


{-| Evaluate a filter condition over every item in order before any projection runs, so the first
error raised matches jmespath's two-pass filter semantics.
-}
filterPass : V -> List V -> List V -> Step (List V)
filterPass condition items kept =
    case items of
        [] ->
            Done (List.reverse kept)

        x :: rest ->
            case evaluate condition x of
                Done result ->
                    if isFalse result then
                        filterPass condition rest kept

                    else
                        filterPass condition rest (x :: kept)

                Failed category ->
                    Failed category

                suspended ->
                    suspended
                        |> andThen
                            (\result ->
                                if isFalse result then
                                    filterPass condition rest kept

                                else
                                    filterPass condition rest (x :: kept)
                            )


filterBase : V -> V -> V -> Step V
filterBase condition right base =
    case base of
        Arr _ items ->
            case filterPass condition items [] of
                Done kept ->
                    projectHelp right kept []

                Failed category ->
                    Failed category

                suspended ->
                    suspended |> andThen (\kept -> projectHelp right kept [])

        _ ->
            Done Null


{-| Projections keep every result except null; undefined (an empty max\_by) is kept.
-}
projectHelp : V -> List V -> List V -> Step V
projectHelp right items acc =
    case items of
        [] ->
            Done (array (List.reverse acc))

        x :: rest ->
            case evaluate right x of
                Done Null ->
                    projectHelp right rest acc

                Done y ->
                    projectHelp right rest (y :: acc)

                Failed category ->
                    Failed category

                suspended ->
                    suspended
                        |> andThen
                            (\y ->
                                case y of
                                    Null ->
                                        projectHelp right rest acc

                                    _ ->
                                        projectHelp right rest (y :: acc)
                            )


flattenValue : V -> V
flattenValue base =
    case base of
        Arr _ items ->
            array
                (List.concatMap
                    (\item ->
                        case item of
                            Arr _ inner ->
                                inner

                            _ ->
                                [ item ]
                    )
                    items
                )

        _ ->
            Null


buildHash : List ( String, V ) -> V
buildHash pairs =
    object (List.foldl (\( k, v ) -> put k v) [] pairs)


array : List V -> V
array items =
    Arr (fresh ()) items


object : List ( String, V ) -> V
object fields =
    Obj (fresh ()) (sortFields fields)



-- NODES


nodeType : List ( String, V ) -> String
nodeType fields =
    case field "type" fields of
        Str t ->
            t

        _ ->
            ""


children : List ( String, V ) -> List V
children fields =
    case field "children" fields of
        Arr _ items ->
            items

        _ ->
            []


{-| `node.children[i]`.
-}
child : Int -> List ( String, V ) -> V
child i fields =
    children fields |> List.drop i |> List.head |> Maybe.withDefault Undefined



-- INTERPRETER


{-| jmespath's `visit`. Pattern-matches `Done` directly on the fast path to keep JS call-stack
depth low across deeply nested expressions (including right-nested projections and filters).
-}
evaluate : V -> V -> Step V
evaluate node value =
    case node of
        Obj _ fields ->
            case nodeType fields of
                "Field" ->
                    case value of
                        Obj _ object_ ->
                            case field (Values.jsString (field "name" fields)) object_ of
                                Undefined ->
                                    Done Null

                                x ->
                                    Done x

                        _ ->
                            Done Null

                "Subexpression" ->
                    case evaluate (child 0 fields) value of
                        Done left ->
                            evaluate (child 1 fields) left

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended |> andThen (evaluate (child 1 fields))

                "IndexExpression" ->
                    case evaluate (child 0 fields) value of
                        Done left ->
                            evaluate (child 1 fields) left

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended |> andThen (evaluate (child 1 fields))

                "Pipe" ->
                    case evaluate (child 0 fields) value of
                        Done left ->
                            evaluate (child 1 fields) left

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended |> andThen (evaluate (child 1 fields))

                "Index" ->
                    case value of
                        Arr _ items ->
                            Done (index (jsNumber (field "value" fields)) items)

                        _ ->
                            Done Null

                "Slice" ->
                    case value of
                        Arr _ items ->
                            slice (children fields) items

                        _ ->
                            Done Null

                "Projection" ->
                    case evaluate (child 0 fields) value of
                        Done (Arr _ items) ->
                            projectHelp (child 1 fields) items []

                        Done _ ->
                            Done Null

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended
                                |> andThen
                                    (\base ->
                                        case base of
                                            Arr _ items ->
                                                projectHelp (child 1 fields) items []

                                            _ ->
                                                Done Null
                                    )

                "ValueProjection" ->
                    case evaluate (child 0 fields) value of
                        Done (Obj _ object_) ->
                            projectHelp (child 1 fields) (List.map Tuple.second object_) []

                        Done _ ->
                            Done Null

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended
                                |> andThen
                                    (\base ->
                                        case base of
                                            Obj _ object_ ->
                                                projectHelp (child 1 fields) (List.map Tuple.second object_) []

                                            _ ->
                                                Done Null
                                    )

                "FilterProjection" ->
                    case evaluate (child 0 fields) value of
                        Done (Arr _ items) ->
                            -- Every condition first, then the projection: the first error is jmespath's.
                            case filterPass (child 2 fields) items [] of
                                Done kept ->
                                    projectHelp (child 1 fields) kept []

                                Failed category ->
                                    Failed category

                                suspended ->
                                    suspended |> andThen (\kept -> projectHelp (child 1 fields) kept [])

                        Done _ ->
                            Done Null

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended |> andThen (filterBase (child 2 fields) (child 1 fields))

                "Comparator" ->
                    case evaluate (child 0 fields) value of
                        Done first ->
                            case evaluate (child 1 fields) value of
                                Done second ->
                                    comparator (field "name" fields) first second

                                Failed category ->
                                    Failed category

                                suspended ->
                                    suspended |> andThen (\second -> comparator (field "name" fields) first second)

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended
                                |> andThen
                                    (\first ->
                                        evaluate (child 1 fields) value
                                            |> andThen (\second -> comparator (field "name" fields) first second)
                                    )

                "Flatten" ->
                    case evaluate (child 0 fields) value of
                        Done base ->
                            Done (flattenValue base)

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended |> map flattenValue

                "Identity" ->
                    Done value

                "Current" ->
                    Done value

                "MultiSelectList" ->
                    case value of
                        Null ->
                            Done Null

                        _ ->
                            case evalChildren (children fields) value [] of
                                Done items ->
                                    Done (array items)

                                Failed category ->
                                    Failed category

                                suspended ->
                                    suspended |> map array

                "MultiSelectHash" ->
                    case value of
                        Null ->
                            Done Null

                        _ ->
                            case evalHashPairs (children fields) value [] of
                                Done pairs ->
                                    Done (buildHash pairs)

                                Failed category ->
                                    Failed category

                                suspended ->
                                    suspended |> map buildHash

                "OrExpression" ->
                    case evaluate (child 0 fields) value of
                        Done matched ->
                            if isFalse matched then
                                evaluate (child 1 fields) value

                            else
                                Done matched

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended
                                |> andThen
                                    (\matched ->
                                        if isFalse matched then
                                            evaluate (child 1 fields) value

                                        else
                                            Done matched
                                    )

                "AndExpression" ->
                    case evaluate (child 0 fields) value of
                        Done first ->
                            if isFalse first then
                                Done first

                            else
                                evaluate (child 1 fields) value

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended
                                |> andThen
                                    (\first ->
                                        if isFalse first then
                                            Done first

                                        else
                                            evaluate (child 1 fields) value
                                    )

                "NotExpression" ->
                    case evaluate (child 0 fields) value of
                        Done result ->
                            Done (Flag (isFalse result))

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended |> map (isFalse >> Flag)

                "Literal" ->
                    Done (field "value" fields)

                "Function" ->
                    case evalChildren (children fields) value [] of
                        Done args ->
                            callFunction (field "name" fields) args

                        Failed category ->
                            Failed category

                        suspended ->
                            suspended |> andThen (callFunction (field "name" fields))

                "ExpressionReference" ->
                    -- jmespath tags the referenced node in place and returns the node itself: the same
                    -- node, so the same identity, each time.
                    case child 0 fields of
                        Obj token refFields ->
                            Done (Obj token (put "jmespathType" (Str "Expref") refFields))

                        _ ->
                            Failed "syntax"

                _ ->
                    Failed "syntax"

        -- `node.type` of a missing node is a TypeError in jmespath.
        _ ->
            Failed "syntax"


{-| `value[index]` for a JS number index, negative from the end, and null for undefined.
-}
index : Float -> List V -> V
index i items =
    let
        length =
            toFloat (List.length items)

        position =
            if i < 0 then
                length + i

            else
                i
    in
    if 0 <= position && position < length && position == toFloat (floor position) then
        case items |> List.drop (floor position) |> List.head of
            Just Undefined ->
                Null

            Just x ->
                x

            Nothing ->
                Null

    else
        Null


comparator : V -> V -> V -> Step V
comparator name first second =
    let
        ordered test =
            Done (Flag (Maybe.withDefault False (Maybe.map test (relation first second))))
    in
    case name of
        Str "EQ" ->
            Done (Flag (strictDeepEqual first second))

        Str "NE" ->
            Done (Flag (not (strictDeepEqual first second)))

        Str "GT" ->
            ordered ((==) GT)

        Str "GTE" ->
            ordered ((/=) LT)

        Str "LT" ->
            ordered ((==) LT)

        Str "LTE" ->
            ordered ((/=) GT)

        _ ->
            Failed "syntax"



-- SLICES
--
-- Elm's `>` and `>=` call NaN greater than everything, so these comparisons use only `<` and `<=`,
-- which are right about NaN, with the operands swapped where JS has `>`.


{-| jmespath's capSliceRange, in JS numbers.
-}
capSliceRange : Float -> Float -> Float -> Float
capSliceRange length actual step =
    if actual < 0 then
        if actual + length < 0 then
            if step < 0 then
                -1

            else
                0

        else
            actual + length

    else if length <= actual then
        if step < 0 then
            length - 1

        else
            length

    else
        actual


{-| `Slice` with jmespath's computeSliceParams: the bounds are parseInt results, so they may be NaN
(`[0:3:-]`) or past 2^53, and the loop reads `value[i]`, which is undefined one past the end.
-}
slice : List V -> List V -> Step V
slice params items =
    let
        part i =
            case params |> List.drop i |> List.head of
                Just (Num x) ->
                    Just x

                _ ->
                    Nothing

        step =
            Maybe.withDefault 1 (part 2)

        values =
            Array.fromList items

        length =
            toFloat (Array.length values)

        bound i default =
            Maybe.withDefault default (Maybe.map (\x -> capSliceRange length x step) (part i))

        start =
            bound 0
                (if step < 0 then
                    length - 1

                 else
                    0
                )

        stop =
            bound 1
                (if step < 0 then
                    -1

                 else
                    length
                )

        ascending =
            0 < step

        loop i acc =
            if (ascending && i < stop) || (not ascending && stop < i) then
                loop (i + step) (Maybe.withDefault Undefined (Array.get (floor i) values) :: acc)

            else
                List.reverse acc
    in
    -- Elm compiles `step == 0` to `!step`, which NaN passes too.
    if step == 0 && not (isNaN step) then
        Failed "invalid-value"

    else
        Done (array (loop start []))



-- RUNTIME


{-| jmespath's TYPE\_\* constants.
-}
type JmesType
    = TypeNumber
    | TypeAny
    | TypeString
    | TypeArray
    | TypeObject
    | TypeBoolean
    | TypeExpref
    | TypeNull
    | TypeArrayNumber
    | TypeArrayString


{-| jmespath's `_getTypeName`; Nothing where its switch falls through (undefined).
-}
typeName : V -> Maybe JmesType
typeName value =
    case value of
        Str _ ->
            Just TypeString

        Num _ ->
            Just TypeNumber

        Arr _ _ ->
            Just TypeArray

        Flag _ ->
            Just TypeBoolean

        Null ->
            Just TypeNull

        Obj _ fields ->
            case field "jmespathType" fields of
                Str "Expref" ->
                    Just TypeExpref

                _ ->
                    Just TypeObject

        Undefined ->
            Nothing


type alias Parameter =
    { types : List JmesType, variadic : Bool }


parameter : List JmesType -> Parameter
parameter types =
    { types = types, variadic = False }


{-| jmespath's function table, without the functions, which `call` dispatches by name.
-}
signature : String -> Maybe (List Parameter)
signature name =
    case name of
        "abs" ->
            Just [ parameter [ TypeNumber ] ]

        "avg" ->
            Just [ parameter [ TypeArrayNumber ] ]

        "ceil" ->
            Just [ parameter [ TypeNumber ] ]

        "contains" ->
            Just [ parameter [ TypeString, TypeArray ], parameter [ TypeAny ] ]

        "ends_with" ->
            Just [ parameter [ TypeString ], parameter [ TypeString ] ]

        "floor" ->
            Just [ parameter [ TypeNumber ] ]

        "length" ->
            Just [ parameter [ TypeString, TypeArray, TypeObject ] ]

        "map" ->
            Just [ parameter [ TypeExpref ], parameter [ TypeArray ] ]

        "max" ->
            Just [ parameter [ TypeArrayNumber, TypeArrayString ] ]

        "merge" ->
            Just [ { types = [ TypeObject ], variadic = True } ]

        "max_by" ->
            Just [ parameter [ TypeArray ], parameter [ TypeExpref ] ]

        "sum" ->
            Just [ parameter [ TypeArrayNumber ] ]

        "starts_with" ->
            Just [ parameter [ TypeString ], parameter [ TypeString ] ]

        "min" ->
            Just [ parameter [ TypeArrayNumber, TypeArrayString ] ]

        "min_by" ->
            Just [ parameter [ TypeArray ], parameter [ TypeExpref ] ]

        "type" ->
            Just [ parameter [ TypeAny ] ]

        "keys" ->
            Just [ parameter [ TypeObject ] ]

        "values" ->
            Just [ parameter [ TypeObject ] ]

        "sort" ->
            Just [ parameter [ TypeArrayString, TypeArrayNumber ] ]

        "sort_by" ->
            Just [ parameter [ TypeArray ], parameter [ TypeExpref ] ]

        "join" ->
            Just [ parameter [ TypeString ], parameter [ TypeArrayString ] ]

        "reverse" ->
            Just [ parameter [ TypeString, TypeArray ] ]

        "to_array" ->
            Just [ parameter [ TypeAny ] ]

        "to_string" ->
            Just [ parameter [ TypeAny ] ]

        "to_number" ->
            Just [ parameter [ TypeAny ] ]

        "not_null" ->
            Just [ { types = [ TypeAny ], variadic = True } ]

        _ ->
            Nothing


typeMatches : Maybe JmesType -> JmesType -> V -> Bool
typeMatches actual expected argValue =
    case ( expected, argValue ) of
        ( TypeAny, _ ) ->
            True

        ( TypeArray, _ ) ->
            actual == Just TypeArray

        ( TypeArrayNumber, Arr _ items ) ->
            List.all (\item -> typeMatches (typeName item) TypeNumber item) items

        ( TypeArrayString, Arr _ items ) ->
            List.all (\item -> typeMatches (typeName item) TypeString item) items

        ( TypeArrayNumber, _ ) ->
            False

        ( TypeArrayString, _ ) ->
            False

        _ ->
            actual == Just expected


{-| `_validateArgs`: the arity first, then each declared argument's type.
-}
validateArgs : List V -> List Parameter -> Maybe String
validateArgs args parameters =
    let
        variadic =
            List.reverse parameters |> List.head |> Maybe.map .variadic |> Maybe.withDefault False

        arityOk =
            if variadic then
                List.length args >= List.length parameters

            else
                List.length args == List.length parameters

        typesOk =
            List.map2 (\arg p -> List.any (\t -> typeMatches (typeName arg) t arg) p.types) args parameters
                |> List.all identity
    in
    if not arityOk then
        Just "invalid-arity"

    else if not typesOk then
        Just "invalid-type"

    else
        Nothing


{-| `callFunction`: the name must be a known function.
-}
callFunction : V -> List V -> Step V
callFunction name args =
    case name of
        Str functionName ->
            case signature functionName of
                Just parameters ->
                    case validateArgs args parameters of
                        Just category ->
                            Failed category

                        Nothing ->
                            call functionName args

                Nothing ->
                    Failed "unknown-function"

        _ ->
            Failed "unknown-function"


{-| A built-in whose arguments `validateArgs` accepted.
-}
call : String -> List V -> Step V
call name args =
    let
        a =
            List.head args |> Maybe.withDefault Undefined

        b =
            List.drop 1 args |> List.head |> Maybe.withDefault Undefined

        items v =
            case v of
                Arr _ xs ->
                    xs

                _ ->
                    []

        text v =
            case v of
                Str s ->
                    s

                _ ->
                    ""

        number v =
            case v of
                Num x ->
                    x

                _ ->
                    0

        total =
            List.foldl (\x sum -> sum + number x) 0 (items a)
    in
    case name of
        "abs" ->
            Done (Num (jsAbs (number a)))

        "avg" ->
            Done (Num (total / toFloat (List.length (items a))))

        "ceil" ->
            Done (Num (toFloat (ceiling (number a))))

        "contains" ->
            contains a b

        "ends_with" ->
            Done (Flag (String.endsWith (text b) (text a)))

        "floor" ->
            Done (Num (toFloat (floor (number a))))

        "length" ->
            Done
                (Num
                    (toFloat
                        (case a of
                            Str s ->
                                String.length s

                            Arr _ xs ->
                                List.length xs

                            _ ->
                                List.length (fieldList a)
                        )
                    )
                )

        "map" ->
            case evalMap a (items b) [] of
                Done mapped ->
                    Done (array mapped)

                Failed category ->
                    Failed category

                suspended ->
                    suspended |> map array

        "max" ->
            extreme True (items a)

        "merge" ->
            Done (object (List.foldl (\arg fields -> List.foldl (\( k, v ) -> put k v) fields (forInEntries arg)) [] args))

        "max_by" ->
            extremeBy True b (items a)

        "sum" ->
            Done (Num total)

        "starts_with" ->
            -- `lastIndexOf(prefix) === 0`, searching from the end: `starts_with('abab', 'ab')` is false.
            Done
                (Flag
                    (if text b == "" then
                        text a == ""

                     else
                        String.startsWith (text b) (text a) && not (String.contains (text b) (String.dropLeft 1 (text a)))
                    )
                )

        "min" ->
            extreme False (items a)

        "min_by" ->
            extremeBy False b (items a)

        "type" ->
            Done
                (case typeName a of
                    Just TypeNumber ->
                        Str "number"

                    Just TypeString ->
                        Str "string"

                    Just TypeArray ->
                        Str "array"

                    Just TypeObject ->
                        Str "object"

                    Just TypeBoolean ->
                        Str "boolean"

                    Just TypeExpref ->
                        Str "expref"

                    Just TypeNull ->
                        Str "null"

                    _ ->
                        Undefined
                )

        "keys" ->
            Done (array (List.map (Tuple.first >> Str) (fieldList a)))

        "values" ->
            Done (array (List.map Tuple.second (fieldList a)))

        "sort" ->
            -- The engine's default sort: elements compared as strings, numbers too.
            Done (array (List.sortWith (\x y -> compare (jsString x) (jsString y)) (items a)))

        "sort_by" ->
            sortBy b (items a)

        "join" ->
            Done (Str (String.join (text a) (List.map text (items b))))

        "reverse" ->
            Done
                (case a of
                    Str s ->
                        -- Code unit by code unit, as jmespath does: surrogate pairs come out reversed.
                        Str (List.range 0 (String.length s - 1) |> List.map (\i -> String.slice i (i + 1) s) |> List.reverse |> String.concat)

                    _ ->
                        array (List.reverse (items a))
                )

        "to_array" ->
            Done
                (case a of
                    Arr _ _ ->
                        a

                    _ ->
                        array [ a ]
                )

        "to_string" ->
            Done
                (case a of
                    Str _ ->
                        a

                    Undefined ->
                        -- JSON.stringify(undefined) is undefined.
                        Undefined

                    _ ->
                        Str (jsonText a)
                )

        "to_number" ->
            Done
                (case a of
                    Num _ ->
                        a

                    Str _ ->
                        if isNaN (jsNumber a) then
                            Null

                        else
                            Num (jsNumber a)

                    _ ->
                        Null
                )

        "not_null" ->
            -- Undefined is not null here: `not_null(max_by(`[]`, &a))` is undefined.
            Done (List.filter ((/=) (Just TypeNull) << typeName) args |> List.head |> Maybe.withDefault Null)

        _ ->
            Failed "unknown-function"


{-| `Math.abs`, for which -0 is +0: Elm's `abs` keeps -0, and -0 + 0 is +0.
-}
jsAbs : Float -> Float
jsAbs x =
    if x < 0 then
        negate x

    else
        x + 0


{-| `for (key in value)` over a value merge did not type-check: an object's own keys, an array's or
a string's indices, and nothing for other primitives, null and undefined.
-}
forInEntries : V -> List ( String, V )
forInEntries value =
    case value of
        Obj _ fields ->
            fields

        Arr _ xs ->
            List.indexedMap (\i x -> ( String.fromInt i, x )) xs

        Str s ->
            List.range 0 (String.length s - 1) |> List.map (\i -> ( String.fromInt i, Str (String.slice i (i + 1) s) ))

        _ ->
            []


{-| contains: `String.prototype.indexOf`, which converts its argument with String(), or
`Array.prototype.indexOf`, which compares with `===`: primitives by value (NaN never, -0 as 0),
arrays and objects by identity, which the host compares.
-}
contains : V -> V -> Step V
contains subject search =
    case subject of
        Str s ->
            Done (Flag (String.contains (jsString search) s))

        Arr _ candidates ->
            case search of
                Arr token _ ->
                    Identical token (List.filterMap identityOf candidates) (Done << Flag)

                Obj token _ ->
                    Identical token (List.filterMap identityOf candidates) (Done << Flag)

                _ ->
                    Done (Flag (List.any (strictlyEqual search) candidates))

        _ ->
            Done (Flag False)


identityOf : V -> Maybe Token
identityOf value =
    case value of
        Arr token _ ->
            Just token

        Obj token _ ->
            Just token

        _ ->
            Nothing


{-| `===` between a primitive and a value.
-}
strictlyEqual : V -> V -> Bool
strictlyEqual primitive value =
    case ( primitive, value ) of
        ( Num x, Num y ) ->
            x == y

        ( Str x, Str y ) ->
            x == y

        ( Flag x, Flag y ) ->
            x == y

        ( Null, Null ) ->
            True

        ( Undefined, Undefined ) ->
            True

        _ ->
            False


{-| max and min: `Math.max.apply`/`Math.min.apply` over numbers, `localeCompare` over strings.
-}
extreme : Bool -> List V -> Step V
extreme isMax values =
    case values of
        [] ->
            Done Null

        (Num _) :: _ ->
            let
                pick x best =
                    case x of
                        Num n ->
                            if isMax then
                                jsMax best n

                            else
                                jsMin best n

                        _ ->
                            best
            in
            Done
                (Num
                    (List.foldl pick
                        (if isMax then
                            -1 / 0

                         else
                            1 / 0
                        )
                        values
                    )
                )

        first :: rest ->
            extremeString isMax first rest


{-| `if (best.localeCompare(x) < 0) best = x` for max, `if (x.localeCompare(best) < 0)` for min,
each comparison asked of the host.
-}
extremeString : Bool -> V -> List V -> Step V
extremeString isMax best rest =
    case rest of
        [] ->
            Done best

        x :: more ->
            let
                ( left, right ) =
                    if isMax then
                        ( best, x )

                    else
                        ( x, best )
            in
            Collate (jsString left)
                (jsString right)
                (\order ->
                    if order == LT then
                        extremeString isMax x more

                    else
                        extremeString isMax best more
                )


{-| `Math.max`: NaN wins, and +0 is larger than -0.
-}
jsMax : Float -> Float -> Float
jsMax a b =
    if isNaN a || isNaN b then
        0 / 0

    else if a < b then
        b

    else if b < a then
        a

    else if 1 / a < 0 then
        b

    else
        a


{-| `Math.min`: NaN wins, and -0 is smaller than +0.
-}
jsMin : Float -> Float -> Float
jsMin a b =
    if isNaN a || isNaN b then
        0 / 0

    else if a < b then
        a

    else if b < a then
        b

    else if 1 / a < 0 then
        a

    else
        b


{-| `createKeyFunction` over the elements in order: each key evaluated and type-checked.
-}
evalKeys : V -> List V -> List ( V, V ) -> Step (List ( V, V ))
evalKeys exprefNode elements acc =
    case elements of
        [] ->
            Done (List.reverse acc)

        element :: rest ->
            case evaluate exprefNode element of
                Done ((Num _) as key) ->
                    evalKeys exprefNode rest (( key, element ) :: acc)

                Done ((Str _) as key) ->
                    evalKeys exprefNode rest (( key, element ) :: acc)

                Done _ ->
                    Failed "invalid-type"

                Failed category ->
                    Failed category

                suspended ->
                    suspended
                        |> andThen
                            (\key ->
                                case key of
                                    Num _ ->
                                        evalKeys exprefNode rest (( key, element ) :: acc)

                                    Str _ ->
                                        evalKeys exprefNode rest (( key, element ) :: acc)

                                    _ ->
                                        Failed "invalid-type"
                            )


pickExtreme : Bool -> List ( V, V ) -> V
pickExtreme isMax keyed =
    let
        better key best =
            relation key best
                == Just
                    (if isMax then
                        GT

                     else
                        LT
                    )

        initial =
            Num
                (if isMax then
                    -1 / 0

                 else
                    1 / 0
                )
    in
    List.foldl
        (\( key, element ) ( best, record ) ->
            if better key best then
                ( key, element )

            else
                ( best, record )
        )
        ( initial, Undefined )
        keyed
        |> Tuple.second


{-| max\_by and min\_by: the first element whose key beats the running one with JS `>` or `<`,
starting from ∓∞, and undefined when none does.
-}
extremeBy : Bool -> V -> List V -> Step V
extremeBy isMax exprefNode elements =
    case evalKeys exprefNode elements [] of
        Done keyed ->
            Done (pickExtreme isMax keyed)

        Failed category ->
            Failed category

        suspended ->
            suspended |> map (pickExtreme isMax)


evalSortKeys : Maybe JmesType -> V -> List V -> List ( V, V ) -> Step (List ( V, V ))
evalSortKeys required exprefNode elements acc =
    case elements of
        [] ->
            Done (List.reverse acc)

        element :: rest ->
            case evaluate exprefNode element of
                Done key ->
                    if typeName key == required then
                        evalSortKeys required exprefNode rest (( key, element ) :: acc)

                    else
                        Failed "invalid-type"

                Failed category ->
                    Failed category

                suspended ->
                    suspended
                        |> andThen
                            (\key ->
                                if typeName key == required then
                                    evalSortKeys required exprefNode rest (( key, element ) :: acc)

                                else
                                    Failed "invalid-type"
                            )


sortKeyed : List ( V, V ) -> V
sortKeyed keyed =
    List.indexedMap (\i ( key, element ) -> ( i, key, element )) keyed
        |> List.sortWith
            (\( i, keyA, _ ) ( j, keyB, _ ) ->
                case relation keyA keyB of
                    Just GT ->
                        GT

                    Just LT ->
                        LT

                    _ ->
                        compare i j
            )
        |> List.map (\( _, _, element ) -> element)
        |> array


sortByAfterFirst : V -> V -> List V -> V -> Step V
sortByAfterFirst exprefNode first rest firstKey =
    let
        required =
            typeName firstKey
    in
    if required /= Just TypeNumber && required /= Just TypeString then
        Failed "invalid-type"

    else
        case evalSortKeys required exprefNode rest [ ( firstKey, first ) ] of
            Done keyed ->
                Done (sortKeyed keyed)

            Failed category ->
                Failed category

            suspended ->
                suspended |> map sortKeyed


{-| sort\_by: the keys in order, each of the first key's type (number or string), then the engine's
sort (Elm's `List.sortWith` is `Array.prototype.sort`) with jmespath's comparator, the index
breaking ties. With NaN keys the comparator is inconsistent, and the engine's algorithm decides the
order, as it does for jmespath.

jmespath evaluates both `exprA` and `exprB` inside its comparator before type-checking either, in
the order the engine compares them; here keys are evaluated and type-checked in element order,
which reports the same error unless a key-evaluation error races with a wrong-type key (a CONTRACT
exclusion).

-}
sortBy : V -> List V -> Step V
sortBy exprefNode elements =
    case elements of
        [] ->
            Done (array [])

        first :: rest ->
            case evaluate exprefNode first of
                Done firstKey ->
                    sortByAfterFirst exprefNode first rest firstKey

                Failed category ->
                    Failed category

                suspended ->
                    suspended |> andThen (sortByAfterFirst exprefNode first rest)
