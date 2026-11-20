module Values exposing
    ( Op
    , Token
    , V(..)
    , field
    , fieldList
    , fresh
    , fromInput
    , isFalse
    , jsNumber
    , jsString
    , jsonText
    , literal
    , opDecoder
    , put
    , relation
    , sortFields
    , strictDeepEqual
    , toOutput
    )

{-| JavaScript values as jmespath 0.16.0 sees them: the JSON document, the values it builds, and
its own AST nodes, which are objects too.

Arrays and objects carry a `Token` for their identity, which jmespath's `contains` compares with
`===`. Elm values have no identity of their own, so the token is a JS object: the one a value was
decoded from, or a fresh empty array made when the parser or the evaluator builds a value. Only the
host can compare tokens.

The host and the program exchange values as flat postfix instruction lists (`Op`), which both sides
read and write with loops, so no nesting depth costs stack at the boundary.

-}

import Json.Decode as D
import Json.Encode as E


type V
    = Undefined
    | Null
    | Flag Bool
    | Num Float
    | Str String
    | Arr Token (List V)
    | Obj Token (List ( String, V ))


type alias Token =
    E.Value


{-| A token no other value has: each call makes a new JS array.
-}
fresh : () -> Token
fresh () =
    E.list never []



-- OBJECTS


{-| `object[key]` on an object's own fields, undefined when it has none of that name.
-}
field : String -> List ( String, V ) -> V
field key fields =
    case fields of
        [] ->
            Undefined

        ( k, v ) :: rest ->
            if k == key then
                v

            else
                field key rest


{-| The fields of an object, none for anything else.
-}
fieldList : V -> List ( String, V )
fieldList value =
    case value of
        Obj _ fields ->
            fields

        _ ->
            []


{-| `object[key] = value` on an object being built: an existing key keeps its place, a new one goes
last. Assigning `__proto__` reaches Object.prototype's setter, which ignores the primitives this
port's domain assigns there, so it adds no key.
-}
put : String -> V -> List ( String, V ) -> List ( String, V )
put key value fields =
    if key == "__proto__" then
        fields

    else if List.any (\( k, _ ) -> k == key) fields then
        List.map
            (\( k, v ) ->
                if k == key then
                    ( k, value )

                else
                    ( k, v )
            )
            fields

    else
        fields ++ [ ( key, value ) ]


{-| JS objects list canonical array-index keys first, in ascending order, then the other keys in
insertion order; a built object's fields get that order once they are all assigned.
-}
sortFields : List ( String, V ) -> List ( String, V )
sortFields fields =
    let
        arrayIndex ( key, _ ) =
            case String.toInt key of
                Just n ->
                    if n >= 0 && n < 4294967295 && String.fromInt n == key then
                        Just n

                    else
                        Nothing

                Nothing ->
                    Nothing

        order a b =
            case ( arrayIndex a, arrayIndex b ) of
                ( Just i, Just j ) ->
                    compare i j

                ( Just _, Nothing ) ->
                    LT

                ( Nothing, Just _ ) ->
                    GT

                ( Nothing, Nothing ) ->
                    EQ
    in
    List.sortWith order fields



-- JAVASCRIPT SEMANTICS


{-| jmespath's `isFalse`: "", false, null, an empty array and an object with no own keys.
-}
isFalse : V -> Bool
isFalse value =
    case value of
        Str s ->
            s == ""

        Flag b ->
            not b

        Null ->
            True

        Arr _ items ->
            List.isEmpty items

        Obj _ fields ->
            List.isEmpty fields

        _ ->
            False


{-| jmespath's `strictDeepEqual`: `===` for primitives (NaN is unequal to itself, -0 equals 0),
arrays by element, and objects by every own key of the first against the second's value for it
(so an undefined value equals a missing key), with no key of the second missing from the first.
Two values with one identity are equal by content too, so tokens need no comparison here.
-}
strictDeepEqual : V -> V -> Bool
strictDeepEqual first second =
    case ( first, second ) of
        ( Arr _ xs, Arr _ ys ) ->
            List.length xs == List.length ys && List.all identity (List.map2 strictDeepEqual xs ys)

        ( Obj _ xs, Obj _ ys ) ->
            List.all (\( k, x ) -> strictDeepEqual x (field k ys)) xs
                && List.all (\( k, _ ) -> List.any (\( j, _ ) -> j == k) xs) ys

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


{-| `String(value)`: numbers as JS prints them, arrays joined with commas (null and undefined as
empty), objects as `[object Object]`.
-}
jsString : V -> String
jsString value =
    case value of
        Undefined ->
            "undefined"

        Null ->
            "null"

        Flag b ->
            if b then
                "true"

            else
                "false"

        Num x ->
            String.fromFloat x

        Str s ->
            s

        Arr _ items ->
            items
                |> List.map
                    (\item ->
                        case item of
                            Null ->
                                ""

                            Undefined ->
                                ""

                            _ ->
                                jsString item
                    )
                |> String.join ","

        Obj _ _ ->
            "[object Object]"


{-| `Number(value)`, the ToNumber of JS's relational operators and of `to_number`'s unary `+`.
-}
jsNumber : V -> Float
jsNumber value =
    case value of
        Num x ->
            x

        Null ->
            0

        Flag b ->
            if b then
                1

            else
                0

        Undefined ->
            nan

        _ ->
            stringToNumber (jsString value)


nan : Float
nan =
    0 / 0


{-| StringToNumber: surrounding JS whitespace (Elm's `String.trim` is JS's `trim`), the empty
string is 0, `0x`/`0o`/`0b` prefixes in either case, else a decimal literal with `Infinity`
(Elm's `String.toFloat` is JS's `+`, after refusing whitespace and the letters x, b and o).
-}
stringToNumber : String -> Float
stringToNumber text =
    let
        s =
            String.trim text

        radix =
            case String.toLower (String.left 2 s) of
                "0x" ->
                    16

                "0o" ->
                    8

                "0b" ->
                    2

                _ ->
                    10
    in
    if s == "" then
        0

    else if radix /= 10 then
        radixToNumber radix (String.dropLeft 2 s)

    else
        String.toFloat s |> Maybe.withDefault nan


{-| A binary, octal or hexadecimal integer, rounded to the nearest double (ties to even) as JS
does: the digits' bits, the leading 53 of them as the significand, the next bit and the rest as
the rounding.
-}
radixToNumber : Int -> String -> Float
radixToNumber radix digits =
    let
        bitsPerDigit =
            case radix of
                16 ->
                    4

                8 ->
                    3

                _ ->
                    1

        digitValue c =
            let
                code =
                    Char.toCode (Char.toLower c)
            in
            if code >= 48 && code <= 57 then
                code - 48

            else if code >= 97 && code <= 102 then
                code - 87

            else
                99

        values =
            List.map digitValue (String.toList digits)

        bitsOf d =
            List.map (\shift -> modBy 2 (d // (2 ^ shift))) (List.range 0 (bitsPerDigit - 1) |> List.reverse)

        bits =
            List.concatMap bitsOf values |> dropZeros

        dropZeros bs =
            case bs of
                0 :: more ->
                    dropZeros more

                _ ->
                    bs

        significand =
            List.take 53 bits |> List.foldl (\b acc -> acc * 2 + toFloat b) 0

        rest =
            List.drop 53 bits

        exponent =
            List.length rest

        roundUp =
            case rest of
                1 :: sticky ->
                    List.any ((==) 1) sticky || modBy 2 (List.foldl (\b _ -> b) 0 (List.take 53 bits)) == 1

                _ ->
                    False
    in
    if String.isEmpty digits || List.any (\d -> d >= radix) values then
        nan

    else
        (if roundUp then
            significand + 1

         else
            significand
        )
            * (2 ^ toFloat exponent)


{-| JS's abstract relational comparison (`<`, `>`): arrays and objects become their `String()`
forms, two strings compare by UTF-16 code unit (Elm's string `compare` is JS's `<`), anything else
compares as numbers, and NaN compares as nothing.
-}
relation : V -> V -> Maybe Order
relation a b =
    let
        toPrimitive v =
            case v of
                Arr _ _ ->
                    Str (jsString v)

                Obj _ _ ->
                    Str (jsString v)

                _ ->
                    v
    in
    case ( toPrimitive a, toPrimitive b ) of
        ( Str x, Str y ) ->
            Just (compare x y)

        ( x, y ) ->
            let
                u =
                    jsNumber x

                w =
                    jsNumber y
            in
            if isNaN u || isNaN w then
                Nothing

            else
                Just (compare u w)


{-| `JSON.stringify(value)` of a value that is not undefined: object keys and strings through Elm's
encoder (JSON.stringify itself), undefined fields left out and undefined array elements as null.
-}
jsonText : V -> String
jsonText value =
    case value of
        Obj _ fields ->
            "{"
                ++ (fields
                        |> List.filterMap
                            (\( k, v ) ->
                                case v of
                                    Undefined ->
                                        Nothing

                                    _ ->
                                        Just (E.encode 0 (E.string k) ++ ":" ++ jsonText v)
                            )
                        |> String.join ","
                   )
                ++ "}"

        Arr _ items ->
            "[" ++ String.join "," (List.map jsonText items) ++ "]"

        Num x ->
            E.encode 0 (E.float x)

        Str s ->
            E.encode 0 (E.string s)

        Flag b ->
            E.encode 0 (E.bool b)

        _ ->
            "null"



-- BOUNDARY


{-| One postfix instruction: a primitive to push, or an array or object to build from the values
pushed last, with the token it carries.
-}
type Op
    = Push V
    | MakeArray Token Int
    | MakeObject Token (List String)


opDecoder : D.Decoder Op
opDecoder =
    D.oneOf
        [ D.map2 MakeArray (D.field "token" D.value) (D.field "array" D.int)
        , D.map2 MakeObject (D.field "token" D.value) (D.field "object" (D.list D.string))
        , D.map Push primitive
        ]


primitive : D.Decoder V
primitive =
    D.oneOf
        [ D.map Str D.string
        , D.map Num D.float
        , D.map Flag D.bool
        , D.null Null
        ]


{-| The input document, rebuilt from the host's postfix instructions with a stack.
-}
fromInput : List Op -> V
fromInput ops =
    build ops []


build : List Op -> List V -> V
build ops stack =
    case ops of
        [] ->
            List.head stack |> Maybe.withDefault Undefined

        (Push v) :: rest ->
            build rest (v :: stack)

        (MakeArray token n) :: rest ->
            let
                ( items, below ) =
                    pop n stack []
            in
            build rest (Arr token items :: below)

        (MakeObject token keys) :: rest ->
            let
                ( values, below ) =
                    pop (List.length keys) stack []
            in
            build rest (Obj token (List.map2 Tuple.pair keys values) :: below)


{-| The top `n` values of a stack, in the order they were pushed, and the stack below them.
-}
pop : Int -> List V -> List V -> ( List V, List V )
pop n stack taken =
    if n <= 0 then
        ( taken, stack )

    else
        case stack of
            v :: below ->
                pop (n - 1) below (v :: taken)

            [] ->
                ( taken, [] )


{-| A JSON literal of the expression, from `JSON.parse` (Elm's `D.decodeString`): its arrays and
objects get the tokens of the objects JSON.parse made, so the literal keeps one identity however
often its node is evaluated.
-}
literal : String -> Result () V
literal text =
    D.decodeString jsonValue text |> Result.mapError (\_ -> ())


jsonValue : D.Decoder V
jsonValue =
    D.oneOf
        [ primitive
        , D.map2 Arr D.value (D.list (D.lazy (\_ -> jsonValue)))
        , D.map2 Obj D.value (D.keyValuePairs (D.lazy (\_ -> jsonValue)))
        ]


type Work
    = Emit V
    | CloseArray Int
    | CloseObject (List String)


{-| A value as postfix instructions for the host: primitives as themselves, `{undefined}`,
`{array: n}` and `{object: keys}`, written with an explicit work stack.
-}
toOutput : V -> E.Value
toOutput value =
    E.list identity (emit [ Emit value ] [])


emit : List Work -> List E.Value -> List E.Value
emit work out =
    case work of
        [] ->
            List.reverse out

        (CloseArray n) :: rest ->
            emit rest (E.object [ ( "array", E.int n ) ] :: out)

        (CloseObject keys) :: rest ->
            emit rest (E.object [ ( "object", E.list E.string keys ) ] :: out)

        (Emit v) :: rest ->
            case v of
                Undefined ->
                    emit rest (E.object [ ( "undefined", E.bool True ) ] :: out)

                Null ->
                    emit rest (E.null :: out)

                Flag b ->
                    emit rest (E.bool b :: out)

                Num x ->
                    emit rest (E.float x :: out)

                Str s ->
                    emit rest (E.string s :: out)

                Arr _ items ->
                    emit (List.map Emit items ++ CloseArray (List.length items) :: rest) out

                Obj _ fields ->
                    emit (List.map (Tuple.second >> Emit) fields ++ CloseObject (List.map Tuple.first fields) :: rest) out
