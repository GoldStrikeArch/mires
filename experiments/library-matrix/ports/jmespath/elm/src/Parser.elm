module Parser exposing (compile)

{-| jmespath's Lexer and Pratt parser. The AST is a `V`, as in jmespath, where nodes are plain
objects that `compile` returns and `&expr` passes around as values; each node gets its own identity.
`Undefined` stands for jmespath's undefined nodes (`a.`, `()`).

The lexer and the operator loop of `expression` are tail calls, so a long chain (`a.a.a…`,
`a || a || …`) costs no stack; only nesting does.

-}

import Array exposing (Array)
import Values exposing (V(..), fresh)


type TokenType
    = EOF
    | UnquotedIdentifier
    | QuotedIdentifier
    | Rbracket
    | Rparen
    | Comma
    | Colon
    | Rbrace
    | Number
    | Current
    | Expref
    | Pipe
    | Or
    | And
    | EQ
    | GT
    | LT
    | GTE
    | LTE
    | NE
    | Flatten
    | Star
    | Filter
    | Dot
    | Not
    | Lbrace
    | Lbracket
    | Lparen
    | Literal


{-| jmespath's `bindingPower` table. It has no entry for Colon and Literal: `rbp < bindingPower[Colon]`
is false, and so is `bindingPower[Literal] < 10` in `_parseProjectionRHS`, which therefore rejects a
literal after a projection.
-}
bindingPower : TokenType -> Maybe Int
bindingPower t =
    case t of
        Colon ->
            Nothing

        Literal ->
            Nothing

        Pipe ->
            Just 1

        Or ->
            Just 2

        And ->
            Just 3

        EQ ->
            Just 5

        GT ->
            Just 5

        LT ->
            Just 5

        GTE ->
            Just 5

        LTE ->
            Just 5

        NE ->
            Just 5

        Flatten ->
            Just 9

        Star ->
            Just 20

        Filter ->
            Just 21

        Dot ->
            Just 40

        Not ->
            Just 45

        Lbrace ->
            Just 50

        Lbracket ->
            Just 55

        Lparen ->
            Just 60

        _ ->
            Just 0


{-| jmespath's TOK\_\* names of the comparators, which name Comparator nodes.
-}
comparatorName : TokenType -> Maybe String
comparatorName t =
    case t of
        EQ ->
            Just "EQ"

        NE ->
            Just "NE"

        GT ->
            Just "GT"

        GTE ->
            Just "GTE"

        LT ->
            Just "LT"

        LTE ->
            Just "LTE"

        _ ->
            Nothing


type alias Token =
    { kind : TokenType, value : V }



-- LEXER


{-| The code unit at `i` as a one-unit string, empty past the end (where jmespath reads undefined).
-}
at : String -> Int -> String
at source i =
    String.slice i (i + 1) source


isAlpha : String -> Bool
isAlpha c =
    (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") || c == "_"


isNum : String -> Bool
isNum c =
    (c >= "0" && c <= "9") || c == "-"


isAlphaNum : String -> Bool
isAlphaNum c =
    isAlpha c || (c >= "0" && c <= "9")


basicToken : String -> Maybe TokenType
basicToken c =
    case c of
        "." ->
            Just Dot

        "*" ->
            Just Star

        "," ->
            Just Comma

        ":" ->
            Just Colon

        "{" ->
            Just Lbrace

        "}" ->
            Just Rbrace

        "]" ->
            Just Rbracket

        "(" ->
            Just Lparen

        ")" ->
            Just Rparen

        "@" ->
            Just Current

        _ ->
            Nothing


{-| The end of a run of characters that pass `test`, from `i`.
-}
skipWhile : (String -> Bool) -> String -> Int -> Int
skipWhile test source i =
    if i < String.length source && test (at source i) then
        skipWhile test source (i + 1)

    else
        i


{-| The end of a quoted body starting at `i`, where a backslash escapes `quote` or another backslash.
-}
skipQuoted : String -> String -> Int -> Int
skipQuoted quote source i =
    if i >= String.length source || at source i == quote then
        i

    else if at source i == "\\" && (at source (i + 1) == "\\" || at source (i + 1) == quote) then
        skipQuoted quote source (i + 2)

    else
        skipQuoted quote source (i + 1)


{-| JS `text.replace(from, to)` with a string pattern: the first occurrence only.
-}
replaceFirst : String -> String -> String -> String
replaceFirst from to text =
    case String.indexes from text of
        i :: _ ->
            String.left i text ++ to ++ String.dropLeft (i + String.length from) text

        [] ->
            text


{-| `parseInt` of a run of digits and dashes: an optional sign, then the leading digits.
-}
parseInt : String -> Float
parseInt run =
    let
        sign =
            if String.startsWith "-" run then
                "-"

            else
                ""

        digits =
            String.dropLeft (String.length sign) run

        leading =
            String.left (skipWhile (\c -> c >= "0" && c <= "9") digits 0) digits
    in
    if leading == "" then
        0 / 0

    else
        String.toFloat (sign ++ leading) |> Maybe.withDefault (0 / 0)


{-| A backtick literal: after JS's `trimLeft` (Elm's `String.trimLeft` is it) and the first
`` \` `` unescaped, JSON when it looks like JSON, else a JSON string of the text.
-}
backtickLiteral : String -> Result () V
backtickLiteral body =
    let
        literalString =
            replaceFirst "\\`" "`" (String.trimLeft body)

        first =
            String.left 1 literalString

        looksLikeJson =
            if literalString == "" then
                False

            else if String.contains first "[{\"" then
                True

            else if literalString == "true" || literalString == "false" || literalString == "null" then
                True

            else if String.contains first "-0123456789" then
                case Values.literal literalString of
                    Ok _ ->
                        True

                    Err () ->
                        False

            else
                False
    in
    if looksLikeJson then
        Values.literal literalString

    else
        Values.literal ("\"" ++ literalString ++ "\"")


tokenize : String -> Result () (Array Token)
tokenize source =
    lex source 0 []


lex : String -> Int -> List Token -> Result () (Array Token)
lex source i tokens =
    if i >= String.length source then
        Ok (Array.fromList (List.reverse (Token EOF (Str "") :: tokens)))

    else
        let
            c =
                at source i
        in
        if isAlpha c then
            let
                end =
                    skipWhile isAlphaNum source (i + 1)
            in
            lex source end (Token UnquotedIdentifier (Str (String.slice i end source)) :: tokens)

        else
            case basicToken c of
                Just t ->
                    lex source (i + 1) (Token t (Str c) :: tokens)

                Nothing ->
                    if isNum c then
                        let
                            end =
                                skipWhile isNum source (i + 1)
                        in
                        lex source end (Token Number (Num (parseInt (String.slice i end source))) :: tokens)

                    else if c == "[" then
                        case at source (i + 1) of
                            "?" ->
                                lex source (i + 2) (Token Filter (Str "[?") :: tokens)

                            "]" ->
                                lex source (i + 2) (Token Flatten (Str "[]") :: tokens)

                            _ ->
                                lex source (i + 1) (Token Lbracket (Str "[") :: tokens)

                    else if c == "\"" then
                        let
                            end =
                                skipQuoted "\"" source (i + 1) + 1
                        in
                        case Values.literal (String.slice i end source) of
                            Ok name ->
                                lex source end (Token QuotedIdentifier name :: tokens)

                            Err () ->
                                Err ()

                    else if c == "'" then
                        -- An unterminated raw string runs to the end of the expression.
                        let
                            end =
                                skipQuoted "'" source (i + 1)
                        in
                        lex source (end + 1) (Token Literal (Str (replaceFirst "\\'" "'" (String.slice (i + 1) end source))) :: tokens)

                    else if c == "`" then
                        let
                            end =
                                skipQuoted "`" source (i + 1)
                        in
                        case backtickLiteral (String.slice (i + 1) end source) of
                            Ok value ->
                                lex source (end + 1) (Token Literal value :: tokens)

                            Err () ->
                                Err ()

                    else if c == "<" || c == ">" || c == "=" || c == "!" then
                        -- A lone `=` makes no token in jmespath (it pushes undefined, and parsing
                        -- fails on it); that syntax error is raised here.
                        case ( c, at source (i + 1) == "=" ) of
                            ( "!", True ) ->
                                lex source (i + 2) (Token NE (Str "!=") :: tokens)

                            ( "!", False ) ->
                                lex source (i + 1) (Token Not (Str "!") :: tokens)

                            ( "<", True ) ->
                                lex source (i + 2) (Token LTE (Str "<=") :: tokens)

                            ( "<", False ) ->
                                lex source (i + 1) (Token LT (Str "<") :: tokens)

                            ( ">", True ) ->
                                lex source (i + 2) (Token GTE (Str ">=") :: tokens)

                            ( ">", False ) ->
                                lex source (i + 1) (Token GT (Str ">") :: tokens)

                            ( _, True ) ->
                                lex source (i + 2) (Token EQ (Str "==") :: tokens)

                            ( _, False ) ->
                                Err ()

                    else if c == " " || c == "\t" || c == "\n" then
                        lex source (i + 1) tokens

                    else if c == "&" then
                        if at source (i + 1) == "&" then
                            lex source (i + 2) (Token And (Str "&&") :: tokens)

                        else
                            lex source (i + 1) (Token Expref (Str "&") :: tokens)

                    else if c == "|" then
                        if at source (i + 1) == "|" then
                            lex source (i + 2) (Token Or (Str "||") :: tokens)

                        else
                            lex source (i + 1) (Token Pipe (Str "|") :: tokens)

                    else
                        Err ()



-- AST NODES


node : String -> List ( String, V ) -> V
node nodeType properties =
    Obj (fresh ()) (( "type", Str nodeType ) :: properties)


identityNode : () -> V
identityNode () =
    node "Identity" []


branch : String -> List V -> V
branch nodeType children =
    node nodeType [ ( "children", Arr (fresh ()) children ) ]



-- PARSER


type alias State =
    { tokens : Array Token, pos : Int }


type alias Parsed =
    Result () ( V, State )


token : Int -> State -> Token
token offset state =
    Array.get (state.pos + offset) state.tokens |> Maybe.withDefault (Token EOF (Str ""))


lookahead : Int -> State -> TokenType
lookahead offset state =
    (token offset state).kind


advance : State -> State
advance state =
    { state | pos = state.pos + 1 }


match : TokenType -> State -> Result () State
match t state =
    if lookahead 0 state == t then
        Ok (advance state)

    else
        Err ()


{-| `compile`: the AST of an expression, which must end at EOF.
-}
compile : String -> Result () V
compile source =
    case tokenize source of
        Err () ->
            Err ()

        Ok tokens ->
            case expression 0 { tokens = tokens, pos = 0 } of
                Ok ( ast, state ) ->
                    if lookahead 0 state == EOF then
                        Ok ast

                    else
                        Err ()

                Err () ->
                    Err ()


expression : Int -> State -> Parsed
expression rbp state =
    case nud (token 0 state) (advance state) of
        Ok ( left, next ) ->
            operators rbp left next

        Err () ->
            Err ()


{-| The `while (rbp < bindingPower[currentToken])` loop of `expression`, as a tail call.
-}
operators : Int -> V -> State -> Parsed
operators rbp left state =
    let
        currentToken =
            lookahead 0 state
    in
    case bindingPower currentToken of
        Just power ->
            if rbp < power then
                case led currentToken left (advance state) of
                    Ok ( next, after ) ->
                        operators rbp next after

                    Err () ->
                        Err ()

            else
                Ok ( left, state )

        Nothing ->
            Ok ( left, state )


{-| A node built from one parsed child.
-}
wrap : (V -> V) -> Parsed -> Parsed
wrap build parsed =
    case parsed of
        Ok ( child, state ) ->
            Ok ( build child, state )

        Err () ->
            Err ()


nud : Token -> State -> Parsed
nud t state =
    case t.kind of
        Literal ->
            Ok ( node "Literal" [ ( "value", t.value ) ], state )

        UnquotedIdentifier ->
            Ok ( node "Field" [ ( "name", t.value ) ], state )

        QuotedIdentifier ->
            if lookahead 0 state == Lparen then
                Err ()

            else
                Ok ( node "Field" [ ( "name", t.value ) ], state )

        Not ->
            wrap (\right -> branch "NotExpression" [ right ]) (expression 45 state)

        Star ->
            if lookahead 0 state == Rbracket then
                Ok ( branch "ValueProjection" [ identityNode (), identityNode () ], state )

            else
                wrap (\right -> branch "ValueProjection" [ identityNode (), right ]) (parseProjectionRHS 20 state)

        Filter ->
            led Filter (identityNode ()) state

        Lbrace ->
            parseMultiselectHash state []

        Flatten ->
            wrap (\right -> branch "Projection" [ branch "Flatten" [ identityNode () ], right ]) (parseProjectionRHS 9 state)

        Lbracket ->
            if lookahead 0 state == Number || lookahead 0 state == Colon then
                parseIndexExpression (identityNode ()) state

            else if lookahead 0 state == Star && lookahead 1 state == Rbracket then
                wrap (\right -> branch "Projection" [ identityNode (), right ]) (parseProjectionRHS 20 (advance (advance state)))

            else
                parseMultiselectList state []

        Current ->
            Ok ( node "Current" [], state )

        Expref ->
            wrap (\child -> branch "ExpressionReference" [ child ]) (expression 0 state)

        Lparen ->
            -- `args[0]`: undefined for `()`.
            case parseArguments False state [] of
                Ok ( first :: _, after ) ->
                    Ok ( first, after )

                Ok ( [], after ) ->
                    Ok ( Undefined, after )

                Err () ->
                    Err ()

        _ ->
            Err ()


led : TokenType -> V -> State -> Parsed
led tokenName left state =
    case tokenName of
        Dot ->
            if lookahead 0 state /= Star then
                wrap (\right -> branch "Subexpression" [ left, right ]) (parseDotRHS 40 state)

            else
                wrap (\right -> branch "ValueProjection" [ left, right ]) (parseProjectionRHS 40 (advance state))

        Pipe ->
            wrap (\right -> branch "Pipe" [ left, right ]) (expression 1 state)

        Or ->
            wrap (\right -> branch "OrExpression" [ left, right ]) (expression 2 state)

        And ->
            wrap (\right -> branch "AndExpression" [ left, right ]) (expression 3 state)

        Lparen ->
            -- `left.name`: a TypeError (syntax) when there is no left node, as after `()`.
            case left of
                Obj _ fields ->
                    case parseArguments True state [] of
                        Ok ( args, after ) ->
                            Ok ( node "Function" [ ( "name", Values.field "name" fields ), ( "children", Arr (fresh ()) args ) ], after )

                        Err () ->
                            Err ()

                _ ->
                    Err ()

        Filter ->
            case expression 0 state of
                Ok ( condition, afterCondition ) ->
                    case match Rbracket afterCondition of
                        Ok afterBracket ->
                            if lookahead 0 afterBracket == Flatten then
                                Ok ( branch "FilterProjection" [ left, identityNode (), condition ], afterBracket )

                            else
                                wrap (\right -> branch "FilterProjection" [ left, right, condition ]) (parseProjectionRHS 21 afterBracket)

                        Err () ->
                            Err ()

                Err () ->
                    Err ()

        Flatten ->
            wrap (\right -> branch "Projection" [ branch "Flatten" [ left ], right ]) (parseProjectionRHS 9 state)

        Lbracket ->
            if lookahead 0 state == Number || lookahead 0 state == Colon then
                parseIndexExpression left state

            else
                case match Star state |> Result.andThen (match Rbracket) of
                    Ok after ->
                        wrap (\right -> branch "Projection" [ left, right ]) (parseProjectionRHS 20 after)

                    Err () ->
                        Err ()

        _ ->
            case comparatorName tokenName of
                Just name ->
                    wrap (\right -> node "Comparator" [ ( "name", Str name ), ( "children", Arr (fresh ()) [ left, right ] ) ]) (expression 5 state)

                Nothing ->
                    Err ()


{-| The argument loops of `(…)` and of a function call. An argument that starts with `@` is the bare
Current node, so `abs(@.a)` fails on the `.` as in jmespath. Calls may separate arguments with
commas; parentheses may not.
-}
parseArguments : Bool -> State -> List V -> Result () ( List V, State )
parseArguments commas state args =
    if lookahead 0 state == Rparen then
        Ok ( List.reverse args, advance state )

    else
        let
            argument =
                if lookahead 0 state == Current then
                    Ok ( node "Current" [], advance state )

                else
                    expression 0 state
        in
        case argument of
            Ok ( value, after ) ->
                parseArguments commas
                    (if commas && lookahead 0 after == Comma then
                        advance after

                     else
                        after
                    )
                    (value :: args)

            Err () ->
                Err ()


{-| `_parseIndexExpression` and `_projectIfSlice` on `left`.
-}
parseIndexExpression : V -> State -> Parsed
parseIndexExpression left state =
    if lookahead 0 state == Colon || lookahead 1 state == Colon then
        case parseSlice state 0 [ Null, Null, Null ] of
            Ok ( slice, after ) ->
                wrap (\right -> branch "Projection" [ branch "IndexExpression" [ left, slice ], right ]) (parseProjectionRHS 20 after)

            Err () ->
                Err ()

    else
        case match Rbracket (advance state) of
            Ok after ->
                Ok ( branch "IndexExpression" [ left, node "Index" [ ( "value", (token 0 state).value ) ] ], after )

            Err () ->
                Err ()


{-| `[start:stop:step]`, each part optional; a later number in one part replaces an earlier one.
-}
parseSlice : State -> Int -> List V -> Parsed
parseSlice state index parts =
    if lookahead 0 state /= Rbracket && index < 3 then
        case lookahead 0 state of
            Colon ->
                parseSlice (advance state) (index + 1) parts

            Number ->
                parseSlice (advance state) index (List.indexedMap (\i p -> if i == index then (token 0 state).value else p) parts)

            _ ->
                Err ()

    else
        case match Rbracket state of
            Ok after ->
                Ok ( branch "Slice" parts, after )

            Err () ->
                Err ()


{-| Undefined (no error) when nothing that can follow a dot follows it, as in jmespath.
-}
parseDotRHS : Int -> State -> Parsed
parseDotRHS rbp state =
    case lookahead 0 state of
        UnquotedIdentifier ->
            expression rbp state

        QuotedIdentifier ->
            expression rbp state

        Star ->
            expression rbp state

        Lbracket ->
            parseMultiselectList (advance state) []

        Lbrace ->
            parseMultiselectHash (advance state) []

        _ ->
            Ok ( Undefined, state )


parseProjectionRHS : Int -> State -> Parsed
parseProjectionRHS rbp state =
    let
        next =
            lookahead 0 state
    in
    if Maybe.withDefault False (Maybe.map (\power -> power < 10) (bindingPower next)) then
        Ok ( identityNode (), state )

    else if next == Lbracket || next == Filter then
        expression rbp state

    else if next == Dot then
        parseDotRHS rbp (advance state)

    else
        Err ()


{-| `[a, b]`; the comma between elements is optional, a trailing one is an error.
-}
parseMultiselectList : State -> List V -> Parsed
parseMultiselectList state expressions =
    if lookahead 0 state == Rbracket then
        Ok ( branch "MultiSelectList" (List.reverse expressions), advance state )

    else
        case expression 0 state of
            Ok ( value, after ) ->
                if lookahead 0 after == Comma then
                    if lookahead 1 after == Rbracket then
                        Err ()

                    else
                        parseMultiselectList (advance after) (value :: expressions)

                else
                    parseMultiselectList after (value :: expressions)

            Err () ->
                Err ()


{-| `{k: v, …}`; as in jmespath, the comma between pairs is optional too.
-}
parseMultiselectHash : State -> List V -> Parsed
parseMultiselectHash state pairs =
    let
        keyToken =
            token 0 state
    in
    if keyToken.kind /= UnquotedIdentifier && keyToken.kind /= QuotedIdentifier then
        Err ()

    else
        case match Colon (advance state) of
            Err () ->
                Err ()

            Ok afterColon ->
                case expression 0 afterColon of
                    Err () ->
                        Err ()

                    Ok ( value, after ) ->
                        let
                            collected =
                                node "KeyValuePair" [ ( "name", keyToken.value ), ( "value", value ) ] :: pairs
                        in
                        if lookahead 0 after == Comma then
                            parseMultiselectHash (advance after) collected

                        else if lookahead 0 after == Rbrace then
                            Ok ( branch "MultiSelectHash" (List.reverse collected), advance after )

                        else
                            parseMultiselectHash after collected
