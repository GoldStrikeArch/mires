module Text exposing (codeUnitAt, inPropertyOrder, processTerm, tokenize)

{-| MiniSearch's two default text functions, `tokenize` (`text.split(/[\n\r\p{Z}\p{P}]+/u)`) and
`processTerm` (`term.toLowerCase()`), and the order in which `Object.keys` lists an object's keys.

Elm strings are JavaScript strings: `String.length`, `String.slice`, `String.left` and
`String.dropLeft` count UTF-16 code units, and `==` and `<` compare code units, which is exactly
what MiniSearch's radix tree and lengths need (CONTRACT.md, "Strings are UTF-16"). Only
elm/core's iterating functions decode, and they decode a high surrogate together with whatever
unit follows it (see `tokenize`).

-}

import Separators


{-| MiniSearch's default tokenizer: the text between maximal runs of separators. Like `split`, an
empty text is one empty token, and a separator at either end leaves an empty token there.

The text is read as code points, a well-formed surrogate pair being one code point and a lone
surrogate one of its own (a `u`-flag `RegExp` reads it so); a lone surrogate is never a
separator. `String.foldl` gives each code point as a `Char` in one native loop, but elm/core's
kernel builds a `Char` from a high surrogate and the next code unit whatever that unit is (and
from `undefined` after the last one), so the scan checks every `Char` that is not a single code
unit:

  - a high surrogate followed by a low one is the pair's code point;
  - otherwise the high surrogate is lone, and the unit it swallowed is classified on its own; when
    that unit is itself a high surrogate, it may pair with the next `Char`, so it is held back
    (`pending`) until the next one is seen.

Trailing high surrogates are split off before the fold, so it never reads past the text.

-}
tokenize : String -> List String
tokenize text =
    let
        length =
            String.length text

        body =
            String.left (length - trailingHighSurrogates text length) text

        scanned =
            resolvePending (String.foldl (step text) (Scan 0 0 False Nothing []) body)

        -- The trailing high surrogates are lone: not separators.
        final =
            nonSeparator (length - scanned.position) scanned
    in
    List.reverse (String.slice final.start length text :: final.tokens)


type alias Scan =
    { position : Int -- code units classified so far
    , start : Int -- where the current token starts
    , separated : Bool -- whether the last code point was a separator
    , pending : Maybe String -- a high surrogate at `position`, not classified yet
    , tokens : List String -- the tokens so far, last first
    }


step : String -> Char -> Scan -> Scan
step text char scan =
    let
        units =
            String.fromChar char
    in
    case scan.pending of
        Just high ->
            if String.length units == 1 && isLowSurrogate units then
                codePoint text (codeOf (high ++ units)) 2 (resolved scan)

            else
                chunk text char units (nonSeparator 1 (resolved scan))

        Nothing ->
            chunk text char units scan


{-| One `Char` of `String.foldl`: one code unit that is not a high surrogate, or a high surrogate
and the unit after it.
-}
chunk : String -> Char -> String -> Scan -> Scan
chunk text char units scan =
    if String.length units == 1 then
        -- A BMP code point, or a lone low surrogate (never a separator).
        codePoint text (Char.toCode char) 1 scan

    else
        let
            next =
                String.dropLeft 1 units
        in
        if isLowSurrogate next then
            codePoint text (Char.toCode char) 2 scan

        else if isHighSurrogate next then
            -- A lone high surrogate, then one that may pair with the next `Char`.
            { position = scan.position + 1, start = scan.start, separated = False, pending = Just next, tokens = scan.tokens }

        else
            -- A lone high surrogate, then a BMP code point.
            codePoint text (codeOf next) 1 (nonSeparator 1 scan)


codePoint : String -> Int -> Int -> Scan -> Scan
codePoint text code width scan =
    if Separators.isSeparator code then
        { position = scan.position + width
        , start = scan.position + width
        , separated = True
        , pending = Nothing
        , tokens =
            if scan.separated then
                scan.tokens

            else
                String.slice scan.start scan.position text :: scan.tokens
        }

    else
        nonSeparator width scan


nonSeparator : Int -> Scan -> Scan
nonSeparator width scan =
    if width == 0 then
        scan

    else
        { position = scan.position + width, start = scan.start, separated = False, pending = scan.pending, tokens = scan.tokens }


resolved : Scan -> Scan
resolved scan =
    { position = scan.position, start = scan.start, separated = scan.separated, pending = Nothing, tokens = scan.tokens }


{-| A held-back high surrogate that nothing followed is lone.
-}
resolvePending : Scan -> Scan
resolvePending scan =
    case scan.pending of
        Just _ ->
            nonSeparator 1 (resolved scan)

        Nothing ->
            scan


trailingHighSurrogates : String -> Int -> Int
trailingHighSurrogates text length =
    let
        count end =
            if end > 0 && isHighSurrogate (String.slice (end - 1) end text) then
                count (end - 1)

            else
                length - end
    in
    count length


{-| The code point of a string holding exactly one code point that does not start with a lone
high surrogate (a BMP unit, or a well-formed pair).
-}
codeOf : String -> Int
codeOf units =
    String.foldl (\char _ -> Char.toCode char) 0 units


{-| The code unit at `index` (`string.charCodeAt(index)`), 0 past the end. `Char.toCode` decodes a
high surrogate together with the unit after it, so a high surrogate is read paired with U+DC00
and the low half taken off again.
-}
codeUnitAt : Int -> String -> Int
codeUnitAt index string =
    let
        unit =
            String.slice index (index + 1) string
    in
    if isHighSurrogate unit then
        (codeOf (unit ++ lowFirst) - 0x00010000) // 0x0400 + 0xD800

    else
        codeOf unit


{-| Whether a one-unit string is a surrogate of each kind: strings compare by code units.
-}
isHighSurrogate : String -> Bool
isHighSurrogate unit =
    unit >= highFirst && unit <= highLast


isLowSurrogate : String -> Bool
isLowSurrogate unit =
    unit >= lowFirst && unit <= lowLast


highFirst : String
highFirst =
    String.fromChar (Char.fromCode 0xD800)


highLast : String
highLast =
    String.fromChar (Char.fromCode 0xDBFF)


lowFirst : String
lowFirst =
    String.fromChar (Char.fromCode 0xDC00)


lowLast : String
lowLast =
    String.fromChar (Char.fromCode 0xDFFF)


{-| MiniSearch's default term processing, `term.toLowerCase()`: `String.toLower` is the host's
`toLowerCase` (full Unicode mappings and Final_Sigma, as the contract requires).
-}
processTerm : String -> String
processTerm =
    String.toLower


{-| The pairs of an insertion-ordered object in the order `Object.keys` lists its keys: the keys
that are array indices first, in ascending numeric order, then every other key in insertion
order (CONTRACT.md, "`Object.keys(match)` order").
-}
inPropertyOrder : List ( String, a ) -> List ( String, a )
inPropertyOrder pairs =
    let
        ( indices, others ) =
            List.foldr
                (\( key, value ) ( numbered, named ) ->
                    case arrayIndex key of
                        Just index ->
                            ( ( index, ( key, value ) ) :: numbered, named )

                        Nothing ->
                            ( numbered, ( key, value ) :: named )
                )
                ( [], [] )
                pairs
    in
    case indices of
        [] ->
            pairs

        _ ->
            List.map Tuple.second (List.sortBy Tuple.first indices) ++ others


{-| The value of a key that is an array index: the canonical decimal spelling of an integer from 0
to 2^32 - 2 (`"10"`, not `"010"`, `"+1"` or `"4294967295"`).
-}
arrayIndex : String -> Maybe Int
arrayIndex key =
    case String.toInt key of
        Just value ->
            if value >= 0 && value <= 4294967294 && String.fromInt value == key then
                Just value

            else
                Nothing

        Nothing ->
            Nothing
