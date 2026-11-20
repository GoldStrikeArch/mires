module Parser exposing (compile)

import Array exposing (Array)
import Char
import Values exposing (V(..), field, fromJson)


type alias Token =
    { k : String, v : V }


type alias State =
    { tokens : Array Token, pos : Int }


type alias Parsed =
    Result String ( V, State )


node k =
    Obj [ ( "type", Str k ) ]


named k name =
    Obj [ ( "type", Str k ), ( "name", name ) ]


leaf k v =
    Obj [ ( "type", Str k ), ( "value", v ) ]


branch k cs =
    Obj [ ( "type", Str k ), ( "children", Arr cs ) ]


replaceFirst needle replacement source =
    case List.head (String.indexes needle source) of
        Just i ->
            String.left i source ++ replacement ++ String.dropLeft (i + String.length needle) source

        Nothing ->
            source


literal s =
    let
        raw =
            replaceFirst "\\`" "`" (String.trimLeft s)

        initial =
            String.left 1 raw

        quoted _ =
            fromJson ("\"" ++ raw ++ "\"")
    in
    if List.member initial [ "[", "{", "\"" ] || List.member raw [ "true", "false", "null" ] then
        fromJson raw

    else if (initial >= "0" && initial <= "9") || initial == "-" then
        case fromJson raw of
            Ok x ->
                Ok x

            Err _ ->
                quoted ()

    else
        quoted ()


tokens src =
    let
        n =
            String.length src

        ch i =
            String.slice i (i + 1) src

        alpha x =
            (x >= "a" && x <= "z") || (x >= "A" && x <= "Z") || x == "_"

        digit x =
            x >= "0" && x <= "9"

        endWhile f i =
            if i < n && f (ch i) then
                endWhile f (i + 1)

            else
                i

        endQuote quote i =
            if i >= n || ch i == quote then
                i

            else if ch i == "\\" && (ch (i + 1) == quote || ch (i + 1) == "\\") then
                endQuote quote (i + 2)

            else
                endQuote quote (i + 1)

        punctuation =
            [ ( "[?", "Filter" )
            , ( "[]", "Flatten" )
            , ( "&&", "And" )
            , ( "||", "Or" )
            , ( "!=", "NE" )
            , ( "==", "EQ" )
            , ( "<=", "LTE" )
            , ( ">=", "GTE" )
            , ( "<", "LT" )
            , ( ">", "GT" )
            , ( "!", "Not" )
            , ( "&", "Expref" )
            , ( "|", "Pipe" )
            , ( "[", "Lbracket" )
            , ( "]", "Rbracket" )
            , ( "{", "Lbrace" )
            , ( "}", "Rbrace" )
            , ( "(", "Lparen" )
            , ( ")", "Rparen" )
            , ( "*", "Star" )
            , ( "@", "Current" )
            , ( ",", "Comma" )
            , ( ".", "Dot" )
            , ( ":", "Colon" )
            ]

        lex i acc =
            if i >= n then
                Ok { tokens = Array.fromList (List.reverse (Token "EOF" Missing :: acc)), pos = 0 }

            else
                let
                    x =
                        ch i

                    next k v j =
                        lex j (Token k v :: acc)
                in
                if alpha x then
                    let
                        e =
                            endWhile (\c -> alpha c || digit c) (i + 1)
                    in
                    next "Field" (Str (String.slice i e src)) e

                else if digit x || x == "-" then
                    let
                        e =
                            endWhile (\c -> c == "-" || digit c) (i + 1)

                        cut =
                            endWhile digit
                                (if x == "-" then
                                    i + 1

                                 else
                                    i
                                )

                        parsed =
                            String.toFloat (String.slice i cut src) |> Maybe.withDefault (0 / 0)
                    in
                    next "Number" (Num parsed) e

                else if List.member x [ "\"", "'", "`" ] then
                    let
                        e =
                            endQuote x (i + 1)

                        inside =
                            String.slice (i + 1) e src

                        val =
                            if x == "\"" then
                                fromJson (String.slice i (e + 1) src)

                            else if x == "'" then
                                Ok (Str (replaceFirst "\\'" "'" inside))

                            else
                                literal inside
                    in
                    val
                        |> Result.andThen
                            (\v ->
                                next
                                    (if x == "\"" then
                                        "Quoted"

                                     else
                                        "Literal"
                                    )
                                    v
                                    (e + 1)
                            )

                else if List.member x [ " ", "\t", "\n" ] then
                    lex (i + 1) acc

                else
                    case List.filter (\( sym, _ ) -> String.slice i (i + String.length sym) src == sym) punctuation |> List.head of
                        Just ( sym, k ) ->
                            next k Missing (i + String.length sym)

                        Nothing ->
                            Err "syntax"
    in
    lex 0 []


token s offset =
    Array.get (s.pos + offset) s.tokens |> Maybe.withDefault (Token "EOF" Missing)


peek s =
    (token s 0).k


advance n s =
    { s | pos = s.pos + n }


power k =
    if k == "Pipe" then
        1

    else if k == "Or" then
        2

    else if k == "And" then
        3

    else if List.member k [ "LT", "LTE", "GT", "GTE", "NE", "EQ" ] then
        5

    else
        case k of
            "Flatten" ->
                9

            "Star" ->
                20

            "Filter" ->
                21

            "Dot" ->
                40

            "Not" ->
                45

            "Lbrace" ->
                50

            "Lbracket" ->
                55

            "Lparen" ->
                60

            _ ->
                0


syntax =
    Err "syntax"


expect k s =
    if peek s == k then
        Ok (advance 1 s)

    else
        syntax


expression rbp s =
    let
        more ( left, rest ) =
            let
                k =
                    peek rest
            in
            if power k > rbp then
                infixExpr k left (advance 1 rest) |> Result.andThen more

            else
                Ok ( left, rest )
    in
    prefix (token s 0) (advance 1 s) |> Result.andThen more


projection rbp s =
    let
        k =
            peek s
    in
    if power k < 10 then
        Ok ( node "Identity", s )

    else if k == "Lbracket" || k == "Filter" then
        expression rbp s

    else if k == "Dot" then
        dot rbp (advance 1 s)

    else
        syntax


dot rbp s =
    case peek s of
        "Lbracket" ->
            multi (advance 1 s) []

        "Lbrace" ->
            hash (advance 1 s) []

        k ->
            if List.member k [ "Field", "Quoted", "Star" ] then
                expression rbp s

            else
                Ok ( Missing, s )


multi s acc =
    if peek s == "Rbracket" then
        Ok ( branch "MultiSelectList" (List.reverse acc), advance 1 s )

    else
        expression 0 s
            |> Result.andThen
                (\( v, t ) ->
                    if peek t == "Comma" then
                        if (token t 1).k == "Rbracket" then
                            syntax

                        else
                            multi (advance 1 t) (v :: acc)

                    else
                        multi t (v :: acc)
                )


hash s acc =
    let
        t =
            token s 0
    in
    if not (List.member t.k [ "Quoted", "Field" ]) then
        syntax

    else
        expect "Colon" (advance 1 s)
            |> Result.andThen (expression 0)
            |> Result.andThen
                (\( v, after ) ->
                    let
                        pairs =
                            Obj [ ( "type", Str "KeyValuePair" ), ( "name", t.v ), ( "value", v ) ] :: acc
                    in
                    if peek after == "Rbrace" then
                        Ok ( branch "MultiSelectHash" (List.reverse pairs), advance 1 after )

                    else
                        hash
                            (if peek after == "Comma" then
                                advance 1 after

                             else
                                after
                            )
                            pairs
                )


bracket left s =
    if peek s == "Colon" || (token s 1).k == "Colon" then
        let
            scan i parts t =
                if peek t == "Rbracket" || i >= 3 then
                    expect "Rbracket" t
                        |> Result.andThen (projection 20)
                        |> Result.map
                            (\( rhs, rest ) ->
                                ( branch "Projection" [ branch "IndexExpression" [ left, branch "Slice" (Array.toList parts) ], rhs ], rest )
                            )

                else if peek t == "Colon" then
                    scan (i + 1) parts (advance 1 t)

                else if peek t == "Number" then
                    scan i (Array.set i (token t 0).v parts) (advance 1 t)

                else
                    syntax
        in
        scan 0 (Array.repeat 3 Null) s

    else
        expect "Rbracket" (advance 1 s) |> Result.map (\t -> ( branch "IndexExpression" [ left, leaf "Index" (token s 0).v ], t ))


args commas s acc =
    if peek s == "Rparen" then
        Ok ( List.reverse acc, advance 1 s )

    else
        let
            item =
                if peek s == "Current" then
                    Ok ( node "Current", advance 1 s )

                else
                    expression 0 s
        in
        item
            |> Result.andThen
                (\( v, t ) ->
                    args commas
                        (if commas && peek t == "Comma" then
                            advance 1 t

                         else
                            t
                        )
                        (v :: acc)
                )


wrap k before parsed =
    Result.map (\( rhs, t ) -> ( branch k (before ++ [ rhs ]), t )) parsed


prefix t s =
    case t.k of
        "Literal" ->
            Ok ( leaf "Literal" t.v, s )

        "Quoted" ->
            if peek s == "Lparen" then
                syntax

            else
                Ok ( named "Field" t.v, s )

        "Field" ->
            Ok ( named "Field" t.v, s )

        "Not" ->
            wrap "NotExpression" [] (expression 45 s)

        "Current" ->
            Ok ( node "Current", s )

        "Expref" ->
            wrap "ExpressionReference" [] (expression 0 s)

        "Star" ->
            wrap "ValueProjection" [ node "Identity" ] (projection 20 s)

        "Filter" ->
            infixExpr t.k (node "Identity") s

        "Flatten" ->
            infixExpr t.k (node "Identity") s

        "Lbrace" ->
            hash s []

        "Lbracket" ->
            if List.member (peek s) [ "Number", "Colon" ] then
                bracket (node "Identity") s

            else if peek s == "Star" && (token s 1).k == "Rbracket" then
                wrap "Projection" [ node "Identity" ] (projection 20 (advance 2 s))

            else
                multi s []

        "Lparen" ->
            args False s [] |> Result.map (\( cs, rest ) -> ( Maybe.withDefault Missing (List.head cs), rest ))

        _ ->
            syntax


infixExpr k left s =
    case k of
        "Dot" ->
            if peek s == "Star" then
                wrap "ValueProjection" [ left ] (projection 40 (advance 1 s))

            else
                wrap "Subexpression" [ left ] (dot 40 s)

        "Or" ->
            wrap "OrExpression" [ left ] (expression 2 s)

        "And" ->
            wrap "AndExpression" [ left ] (expression 3 s)

        "Pipe" ->
            wrap k [ left ] (expression 1 s)

        "Lparen" ->
            if left == Missing then
                syntax

            else
                args True s [] |> Result.map (\( cs, t ) -> ( Obj [ ( "type", Str "Function" ), ( "name", field "name" left ), ( "children", Arr cs ) ], t ))

        "Filter" ->
            expression 0 s
                |> Result.andThen
                    (\( cond, t ) ->
                        expect "Rbracket" t
                            |> Result.andThen
                                (\rest ->
                                    (if peek rest == "Flatten" then
                                        Ok ( node "Identity", rest )

                                     else
                                        projection 21 rest
                                    )
                                        |> Result.map (\( rhs, final ) -> ( branch "FilterProjection" [ left, rhs, cond ], final ))
                                )
                    )

        "Flatten" ->
            wrap "Projection" [ branch k [ left ] ] (projection 9 s)

        "Lbracket" ->
            if List.member (peek s) [ "Number", "Colon" ] then
                bracket left s

            else
                expect "Star" s |> Result.andThen (expect "Rbracket") |> Result.andThen (projection 20) |> wrap "Projection" [ left ]

        _ ->
            if power k == 5 then
                expression 5 s |> Result.map (\( rhs, t ) -> ( Obj [ ( "type", Str "Comparator" ), ( "name", Str k ), ( "children", Arr [ left, rhs ] ) ], t ))

            else
                syntax


compile s =
    tokens s |> Result.andThen (expression 0)
        |> Result.andThen
            (\( v, t ) ->
                if peek t == "EOF" then
                    Ok v

                else
                    syntax
            )
