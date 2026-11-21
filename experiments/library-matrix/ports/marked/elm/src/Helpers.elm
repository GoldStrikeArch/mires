module Helpers exposing
    ( cleanUrl
    , collapseSpaces
    , decodeNumericCharacterReferences
    , escapeHtmlEntities
    , expandTabs
    , findClosingParenSpan
    , isBlankLine
    , isBlankLineSpan
    , isEntityAt
    , normalizeLabel
    , normalizeNewlines
    , pushEscapedHtml
    , rtrimChar
    , splitCells
    , trimTrailingBlankLines
    , unescapeAnyPunctuation
    , unescapeLinkBrackets
    )

{-| Exact ports of `src/helpers.ts` from marked 18.1.1.
-}

import Array exposing (Array)
import Bitwise
import Unicode
    exposing
        ( Src
        , at
        , charWidth
        , fromString
        , hasCode
        , isAsciiAlnum
        , isAsciiDigit
        , isAsciiHexDigit
        , isAsciiWord
        , isJsSpace
        , isPunct
        )


{-| Checks whether `codes[ampPos..len)` starts with an HTML entity reference matching
`&(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);`.
-}
isEntityAt : Array Int -> Int -> Int -> Bool
isEntityAt codes len ampPos =
    if ampPos + 2 - len >= 0 || at ampPos codes /= 38 then
        False

    else
        let
            i =
                ampPos + 1

            c1 =
                at i codes
        in
        if c1 == 35 then
            -- '#'
            if i + 1 - len >= 0 then
                False

            else
                let
                    c2 =
                        at (i + 1) codes
                in
                if c2 == 120 || c2 == 88 then
                    -- 'x' or 'X'
                    scanHexEntity codes len (i + 2) 0

                else
                    scanDecEntity codes len (i + 1) 0

        else
            scanNamedEntity codes len i i


scanHexEntity : Array Int -> Int -> Int -> Int -> Bool
scanHexEntity codes len j digits =
    if j - len < 0 && digits < 6 && isAsciiHexDigit (at j codes) then
        scanHexEntity codes len (j + 1) (digits + 1)

    else
        digits >= 1 && j - len < 0 && at j codes == 59


scanDecEntity : Array Int -> Int -> Int -> Int -> Bool
scanDecEntity codes len j digits =
    if j - len < 0 && digits < 7 && isAsciiDigit (at j codes) then
        scanDecEntity codes len (j + 1) (digits + 1)

    else
        digits >= 1 && j - len < 0 && at j codes == 59


scanNamedEntity : Array Int -> Int -> Int -> Int -> Bool
scanNamedEntity codes len start j =
    if j - len < 0 && isAsciiWord (at j codes) then
        scanNamedEntity codes len start (j + 1)

    else
        j - start > 0 && j - len < 0 && at j codes == 59


{-| Exact port of `escapeHtmlEntities(html, encode)` in `helpers.ts`.
-}
escapeHtmlEntities : Bool -> String -> String
escapeHtmlEntities encode s =
    if hasHtmlSpecial s then
        let
            src =
                fromString s
        in
        escapeLoop encode src 0 0 []
            |> List.reverse
            |> String.concat

    else
        s


{-| Pushes HTML-escaped chunks of `s` onto a reversed output accumulator `acc`.
-}
pushEscapedHtml : Bool -> String -> List String -> List String
pushEscapedHtml encode s acc =
    if String.isEmpty s then
        acc

    else if hasHtmlSpecial s then
        let
            src =
                fromString s
        in
        escapeLoop encode src 0 0 acc

    else
        s :: acc


hasHtmlSpecial : String -> Bool
hasHtmlSpecial s =
    String.contains "&" s
        || String.contains "<" s
        || String.contains ">" s
        || String.contains "\"" s
        || String.contains "'" s


escapeLoop : Bool -> Src -> Int -> Int -> List String -> List String
escapeLoop encode src last i acc =
    if i - src.len >= 0 then
        if last - src.len < 0 then
            String.slice last src.len src.str :: acc

        else
            acc

    else
        let
            c =
                at i src.codes
        in
        if c == 38 then
            if encode || not (isEntityAt src.codes src.len i) then
                let
                    acc1 =
                        if i - last > 0 then
                            String.slice last i src.str :: acc

                        else
                            acc
                in
                escapeLoop encode src (i + 1) (i + 1) ("&amp;" :: acc1)

            else
                escapeLoop encode src last (i + 1) acc

        else if c == 60 then
            let
                acc1 =
                    if i - last > 0 then
                        String.slice last i src.str :: acc

                    else
                        acc
            in
            escapeLoop encode src (i + 1) (i + 1) ("&lt;" :: acc1)

        else if c == 62 then
            let
                acc1 =
                    if i - last > 0 then
                        String.slice last i src.str :: acc

                    else
                        acc
            in
            escapeLoop encode src (i + 1) (i + 1) ("&gt;" :: acc1)

        else if c == 34 then
            let
                acc1 =
                    if i - last > 0 then
                        String.slice last i src.str :: acc

                    else
                        acc
            in
            escapeLoop encode src (i + 1) (i + 1) ("&quot;" :: acc1)

        else if c == 39 then
            let
                acc1 =
                    if i - last > 0 then
                        String.slice last i src.str :: acc

                    else
                        acc
            in
            escapeLoop encode src (i + 1) (i + 1) ("&#39;" :: acc1)

        else
            escapeLoop encode src last (i + charWidth c) acc


{-| Attempts to parse a numeric character reference at `codes[pos..len)` matching
`/&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));/`. Returns `Just ( endIdx, codePoint )`.
-}
parseNumericCharRef : Array Int -> Int -> Int -> Maybe ( Int, Int )
parseNumericCharRef codes len pos =
    if pos + 3 - len >= 0 || at pos codes /= 38 || at (pos + 1) codes /= 35 then
        Nothing

    else
        let
            i =
                pos + 2

            c =
                at i codes
        in
        if c == 120 || c == 88 then
            parseHexRef codes len (i + 1) 0 0

        else
            parseDecRef codes len i 0 0


parseHexRef : Array Int -> Int -> Int -> Int -> Int -> Maybe ( Int, Int )
parseHexRef codes len j digits val =
    if j - len < 0 && digits < 6 then
        let
            c =
                at j codes
        in
        if c >= 48 && c <= 57 then
            parseHexRef codes len (j + 1) (digits + 1) (val * 16 + (c - 48))

        else if c >= 65 && c <= 70 then
            parseHexRef codes len (j + 1) (digits + 1) (val * 16 + (c - 55))

        else if c >= 97 && c <= 102 then
            parseHexRef codes len (j + 1) (digits + 1) (val * 16 + (c - 87))

        else if digits >= 1 && c == 59 then
            Just ( j + 1, val )

        else
            Nothing

    else if digits >= 1 && j - len < 0 && at j codes == 59 then
        Just ( j + 1, val )

    else
        Nothing


parseDecRef : Array Int -> Int -> Int -> Int -> Int -> Maybe ( Int, Int )
parseDecRef codes len j digits val =
    if j - len < 0 && digits < 7 then
        let
            c =
                at j codes
        in
        if c >= 48 && c <= 57 then
            parseDecRef codes len (j + 1) (digits + 1) (val * 10 + (c - 48))

        else if digits >= 1 && c == 59 then
            Just ( j + 1, val )

        else
            Nothing

    else if digits >= 1 && j - len < 0 && at j codes == 59 then
        Just ( j + 1, val )

    else
        Nothing


hasAnyNcr : Array Int -> Int -> Int -> Bool
hasAnyNcr codes len i =
    if i + 3 - len >= 0 then
        False

    else if at i codes == 38 && at (i + 1) codes == 35 then
        case parseNumericCharRef codes len i of
            Just _ ->
                True

            Nothing ->
                hasAnyNcr codes len (i + 1)

    else
        hasAnyNcr codes len (i + 1)


{-| Exact port of `decodeNumericCharacterReferences(text)` in `helpers.ts`.
-}
decodeNumericCharacterReferences : String -> String
decodeNumericCharacterReferences text =
    if not (String.contains "&#" text) then
        text

    else
        let
            src =
                fromString text
        in
        if not (hasAnyNcr src.codes src.len 0) then
            text

        else
            decodeNcrLoop src 0 0 []
                |> List.reverse
                |> String.concat


decodeNcrLoop : Src -> Int -> Int -> List String -> List String
decodeNcrLoop src last i acc =
    if i - src.len >= 0 then
        if last - src.len < 0 then
            String.slice last src.len src.str :: acc

        else
            acc

    else
        let
            c =
                at i src.codes
        in
        if c == 38 then
            case parseNumericCharRef src.codes src.len i of
                Just ( endIdx, code ) ->
                    let
                        acc1 =
                            if i - last > 0 then
                                String.slice last i src.str :: acc

                            else
                                acc

                        decoded =
                            if code == 0 || code > 0x0010FFFF || (code >= 0xD800 && code <= 0xDFFF) then
                                "\u{FFFD}"

                            else if code == 38 then
                                "&amp;"

                            else
                                String.fromChar (Char.fromCode code)
                    in
                    decodeNcrLoop src endIdx endIdx (decoded :: acc1)

                Nothing ->
                    if not (isEntityAt src.codes src.len i) then
                        let
                            acc1 =
                                if i - last > 0 then
                                    String.slice last i src.str :: acc

                                else
                                    acc
                        in
                        decodeNcrLoop src (i + 1) (i + 1) ("&amp;" :: acc1)

                    else
                        decodeNcrLoop src last (i + 1) acc

        else
            decodeNcrLoop src last (i + charWidth c) acc


{-| Exact port of `cleanUrl` (`encodeURI(href).replace(/%25/g, '%')`, returning `Nothing` on
`URIError` from lone UTF-16 surrogates).
Keeps ASCII bytes in `encodeURI`'s unescaped set plus `%` verbatim and percent-encodes
all other UTF-8 bytes as uppercase `%XX`.
-}
cleanUrl : String -> Maybe String
cleanUrl href =
    let
        step ch st =
            let
                cp =
                    Char.toCode ch

                w =
                    charWidth cp
            in
            if st.invalid || (cp >= 0xD800 && cp <= 0xDFFF) || cp > 0x0010FFFF then
                { lastIdx = st.lastIdx, curIdx = st.curIdx + w, acc = st.acc, invalid = True }

            else if isCleanUrlKeep cp then
                { lastIdx = st.lastIdx, curIdx = st.curIdx + 1, acc = st.acc, invalid = False }

            else
                let
                    acc1 =
                        if st.curIdx - st.lastIdx > 0 then
                            String.slice st.lastIdx st.curIdx href :: st.acc

                        else
                            st.acc

                    acc2 =
                        pushPercentEncodedCp cp acc1

                    nextIdx =
                        st.curIdx + w
                in
                { lastIdx = nextIdx, curIdx = nextIdx, acc = acc2, invalid = False }

        final =
            String.foldl step { lastIdx = 0, curIdx = 0, acc = [], invalid = False } href
    in
    if final.invalid || final.curIdx - String.length href /= 0 then
        Nothing

    else if final.lastIdx == 0 && List.isEmpty final.acc then
        Just href

    else
        let
            completed =
                if final.curIdx - final.lastIdx > 0 then
                    String.slice final.lastIdx final.curIdx href :: final.acc

                else
                    final.acc
        in
        Just (String.concat (List.reverse completed))


isCleanUrlKeep : Int -> Bool
isCleanUrlKeep cp =
    if cp >= 0x80 then
        False

    else if isAsciiAlnum cp then
        True

    else
        case cp of
            33 ->
                -- '!'
                True

            35 ->
                -- '#'
                True

            36 ->
                -- '$'
                True

            37 ->
                -- '%'
                True

            38 ->
                -- '&'
                True

            39 ->
                -- '\''
                True

            40 ->
                -- '('
                True

            41 ->
                -- ')'
                True

            42 ->
                -- '*'
                True

            43 ->
                -- '+'
                True

            44 ->
                -- ','
                True

            45 ->
                -- '-'
                True

            46 ->
                -- '.'
                True

            47 ->
                -- '/'
                True

            58 ->
                -- ':'
                True

            59 ->
                -- ';'
                True

            61 ->
                -- '='
                True

            63 ->
                -- '?'
                True

            64 ->
                -- '@'
                True

            95 ->
                -- '_'
                True

            126 ->
                -- '~'
                True

            _ ->
                False


pushPercentEncodedCp : Int -> List String -> List String
pushPercentEncodedCp cp acc =
    if cp < 0x80 then
        percentByte cp :: acc

    else if cp < 0x0800 then
        let
            b0 =
                Bitwise.or 0xC0 (Bitwise.shiftRightZfBy 6 cp)

            b1 =
                Bitwise.or 0x80 (Bitwise.and cp 0x3F)
        in
        percentByte b1 :: percentByte b0 :: acc

    else if cp < 0x00010000 then
        let
            b0 =
                Bitwise.or 0xE0 (Bitwise.shiftRightZfBy 12 cp)

            b1 =
                Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightZfBy 6 cp) 0x3F)

            b2 =
                Bitwise.or 0x80 (Bitwise.and cp 0x3F)
        in
        percentByte b2 :: percentByte b1 :: percentByte b0 :: acc

    else
        let
            b0 =
                Bitwise.or 0xF0 (Bitwise.shiftRightZfBy 18 cp)

            b1 =
                Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightZfBy 12 cp) 0x3F)

            b2 =
                Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightZfBy 6 cp) 0x3F)

            b3 =
                Bitwise.or 0x80 (Bitwise.and cp 0x3F)
        in
        percentByte b3 :: percentByte b2 :: percentByte b1 :: percentByte b0 :: acc


percentByte : Int -> String
percentByte b =
    "%" ++ hexDigit (Bitwise.shiftRightZfBy 4 b) ++ hexDigit (Bitwise.and b 0x0F)


hexDigit : Int -> String
hexDigit n =
    case n of
        0 ->
            "0"

        1 ->
            "1"

        2 ->
            "2"

        3 ->
            "3"

        4 ->
            "4"

        5 ->
            "5"

        6 ->
            "6"

        7 ->
            "7"

        8 ->
            "8"

        9 ->
            "9"

        10 ->
            "A"

        11 ->
            "B"

        12 ->
            "C"

        13 ->
            "D"

        14 ->
            "E"

        _ ->
            "F"


{-| Exact port of `splitCells(tableRow, count)` in `helpers.ts`.
-}
splitCells : Maybe Int -> String -> List String
splitCells maybeCount tableRow =
    let
        spaced =
            if String.contains "|" tableRow then
                insertSpaceBeforeUnescapedPipes (fromString tableRow) 0 0 []
                    |> List.reverse
                    |> String.concat

            else
                tableRow

        rawCells =
            String.split " |" spaced

        trimmedHead =
            case rawCells of
                first :: rest ->
                    if String.isEmpty (String.trim first) then
                        rest

                    else
                        rawCells

                [] ->
                    []

        trimmedBoth =
            dropTrailingEmptyCell trimmedHead

        cleaned =
            List.foldl
                (\cell acc ->
                    let
                        t =
                            String.trim cell

                        u =
                            if String.contains "\\|" t then
                                String.replace "\\|" "|" t

                            else
                                t
                    in
                    u :: acc
                )
                []
                trimmedBoth
                |> List.reverse
    in
    case maybeCount of
        Just target ->
            if target > 0 then
                adjustCellCount target cleaned

            else
                cleaned

        Nothing ->
            cleaned


insertSpaceBeforeUnescapedPipes : Src -> Int -> Int -> List String -> List String
insertSpaceBeforeUnescapedPipes src last i acc =
    if i - src.len >= 0 then
        if last - src.len < 0 then
            String.slice last src.len src.str :: acc

        else
            acc

    else if at i src.codes == 124 then
        -- '|'
        if isPrecededByOddBackslashes src.codes i then
            insertSpaceBeforeUnescapedPipes src last (i + 1) acc

        else
            let
                acc1 =
                    if i - last > 0 then
                        String.slice last i src.str :: acc

                    else
                        acc
            in
            insertSpaceBeforeUnescapedPipes src (i + 1) (i + 1) (" |" :: acc1)

    else
        insertSpaceBeforeUnescapedPipes src last (i + 1) acc


isPrecededByOddBackslashes : Array Int -> Int -> Bool
isPrecededByOddBackslashes codes idx =
    countBackslashesLoop codes idx False


countBackslashesLoop : Array Int -> Int -> Bool -> Bool
countBackslashesLoop codes curr escaped =
    if curr > 0 && at (curr - 1) codes == 92 then
        countBackslashesLoop codes (curr - 1) (not escaped)

    else
        escaped


dropTrailingEmptyCell : List String -> List String
dropTrailingEmptyCell cells =
    case List.reverse cells of
        lastCell :: revRest ->
            if String.isEmpty (String.trim lastCell) then
                List.reverse revRest

            else
                cells

        [] ->
            []


adjustCellCount : Int -> List String -> List String
adjustCellCount target cells =
    adjustCellCountLoop target cells []


adjustCellCountLoop : Int -> List String -> List String -> List String
adjustCellCountLoop remaining cells acc =
    if remaining <= 0 then
        List.reverse acc

    else
        case cells of
            c :: rest ->
                adjustCellCountLoop (remaining - 1) rest (c :: acc)

            [] ->
                adjustCellCountLoop (remaining - 1) [] ("" :: acc)


{-| Remove trailing ASCII code units equal to `ch` (`rtrim(str, c)` in `helpers.ts`).
-}
rtrimChar : Int -> String -> String
rtrimChar ch s =
    let
        chStr =
            String.fromChar (Char.fromCode ch)
    in
    if not (String.endsWith chStr s) then
        s

    else
        let
            src =
                fromString s

            end =
                rtrimLoop src.codes ch src.len
        in
        String.slice 0 end s


rtrimLoop : Array Int -> Int -> Int -> Int
rtrimLoop codes ch end =
    if end > 0 && at (end - 1) codes - ch == 0 then
        rtrimLoop codes ch (end - 1)

    else
        end


{-| Checks `/^[ \t]*$/`.
-}
isBlankLine : String -> Bool
isBlankLine line =
    if String.isEmpty line then
        True

    else
        let
            src =
                fromString line
        in
        isBlankLineSpan src.codes 0 src.len


isBlankLineSpan : Array Int -> Int -> Int -> Bool
isBlankLineSpan codes pos end =
    if pos - end >= 0 then
        True

    else
        let
            c =
                at pos codes
        in
        if c == 32 || c == 9 then
            isBlankLineSpan codes (pos + 1) end

        else
            False


{-| Exact port of `trimTrailingBlankLines` in `helpers.ts`.
-}
trimTrailingBlankLines : String -> String
trimTrailingBlankLines s =
    let
        lines =
            String.split "\n" s

        total =
            List.length lines

        lastNonBlank =
            findLastNonBlankLine (Array.fromList lines) (total - 1)
    in
    if total - lastNonBlank <= 2 then
        s

    else if lastNonBlank < 0 then
        ""

    else
        List.take (lastNonBlank + 1) lines
            |> String.join "\n"


findLastNonBlankLine : Array String -> Int -> Int
findLastNonBlankLine arr idx =
    if idx < 0 then
        -1

    else
        case Array.get idx arr of
            Just line ->
                if isBlankLine line then
                    findLastNonBlankLine arr (idx - 1)

                else
                    idx

            Nothing ->
                -1


{-| Exact port of `normalizeLabel(label)`:
`label.trim().toLowerCase().toUpperCase().toLowerCase()`.
-}
normalizeLabel : String -> String
normalizeLabel label =
    label
        |> String.trim
        |> String.toLower
        |> String.toUpper
        |> String.toLower


type alias CollapseState =
    { lastIdx : Int
    , curIdx : Int
    , inSpace : Bool
    , acc : List String
    }


{-| Collapses runs of JS whitespace (`/\s+/g`) to a single `' '`.
-}
collapseSpaces : String -> String
collapseSpaces s =
    let
        step ch st =
            let
                cp =
                    Char.toCode ch

                w =
                    charWidth cp
            in
            if isJsSpace cp then
                if st.inSpace then
                    { lastIdx = st.curIdx + w, curIdx = st.curIdx + w, inSpace = True, acc = st.acc }

                else
                    let
                        acc1 =
                            if st.curIdx - st.lastIdx > 0 then
                                String.slice st.lastIdx st.curIdx s :: st.acc

                            else
                                st.acc
                    in
                    { lastIdx = st.curIdx + w, curIdx = st.curIdx + w, inSpace = True, acc = " " :: acc1 }

            else
                { lastIdx = st.lastIdx, curIdx = st.curIdx + w, inSpace = False, acc = st.acc }

        finalSt =
            String.foldl step { lastIdx = 0, curIdx = 0, inSpace = False, acc = [] } s
    in
    if finalSt.lastIdx == 0 && List.isEmpty finalSt.acc then
        s

    else
        let
            completed =
                if finalSt.curIdx - finalSt.lastIdx > 0 then
                    String.slice finalSt.lastIdx finalSt.curIdx s :: finalSt.acc

                else
                    finalSt.acc
        in
        String.concat (List.reverse completed)


{-| Exact port of `findClosingBracket(str, "()")` in `helpers.ts` operating on a span
`codes[start..end)`. Returns UTF-16 offset relative to `start`, `-2` for unmatched `(`,
or `-1` if balanced / no parens.
-}
findClosingParenSpan : Array Int -> Int -> Int -> Int
findClosingParenSpan codes start end =
    if not (hasCode 40 codes start end || hasCode 41 codes start end) then
        -1

    else
        findClosingParenLoop codes start end start 0


findClosingParenLoop : Array Int -> Int -> Int -> Int -> Int -> Int
findClosingParenLoop codes start end i level =
    if i - end >= 0 then
        if level > 0 then
            -2

        else
            -1

    else
        let
            c =
                at i codes
        in
        if c == 92 then
            -- '\\': skip next character (UTF-16 unit or full scalar)
            let
                nextI =
                    if i + 1 - end < 0 then
                        i + 1 + charWidth (at (i + 1) codes)

                    else
                        i + 1
            in
            findClosingParenLoop codes start end nextI level

        else if c == 40 then
            -- '('
            findClosingParenLoop codes start end (i + 1) (level + 1)

        else if c == 41 then
            -- ')'
            let
                nextLevel =
                    level - 1
            in
            if nextLevel < 0 then
                i - start

            else
                findClosingParenLoop codes start end (i + 1) nextLevel

        else
            findClosingParenLoop codes start end (i + charWidth c) level


type alias TabState =
    { col : Int
    , lastIdx : Int
    , curIdx : Int
    , acc : List String
    }


{-| Exact port of `expandTabs(line, indent)` in `helpers.ts`.
Counts Unicode scalar values starting at `indent`, matching JS `for (const char of line)`.
-}
expandTabs : Int -> String -> String
expandTabs indent line =
    if not (String.contains "\t" line) then
        line

    else
        let
            step ch st =
                let
                    cp =
                        Char.toCode ch

                    w =
                        charWidth cp
                in
                if cp == 9 then
                    let
                        added =
                            4 - modBy 4 st.col

                        acc1 =
                            if st.curIdx - st.lastIdx > 0 then
                                String.slice st.lastIdx st.curIdx line :: st.acc

                            else
                                st.acc

                        spaces =
                            case added of
                                1 ->
                                    " "

                                2 ->
                                    "  "

                                3 ->
                                    "   "

                                _ ->
                                    "    "
                    in
                    { col = st.col + added
                    , lastIdx = st.curIdx + 1
                    , curIdx = st.curIdx + 1
                    , acc = spaces :: acc1
                    }

                else
                    { col = st.col + 1
                    , lastIdx = st.lastIdx
                    , curIdx = st.curIdx + w
                    , acc = st.acc
                    }

            finalSt =
                String.foldl step { col = indent, lastIdx = 0, curIdx = 0, acc = [] } line

            completed =
                if finalSt.curIdx - finalSt.lastIdx > 0 then
                    String.slice finalSt.lastIdx finalSt.curIdx line :: finalSt.acc

                else
                    finalSt.acc
        in
        String.concat (List.reverse completed)


{-| Replaces `/\\([\p{P}\p{S}])/gu` with `$1` (`rules.inline.anyPunctuation`).
-}
unescapeAnyPunctuation : String -> String
unescapeAnyPunctuation s =
    if not (String.contains "\\" s) then
        s

    else
        let
            src =
                fromString s
        in
        unescapeAnyPunctLoop src 0 0 []
            |> List.reverse
            |> String.concat


unescapeAnyPunctLoop : Src -> Int -> Int -> List String -> List String
unescapeAnyPunctLoop src last i acc =
    if i - src.len >= 0 then
        if last - src.len < 0 then
            String.slice last src.len src.str :: acc

        else
            acc

    else if at i src.codes == 92 && i + 1 - src.len < 0 then
        let
            nextCp =
                at (i + 1) src.codes

            w =
                charWidth nextCp
        in
        if isPunct nextCp then
            let
                acc1 =
                    if i - last > 0 then
                        String.slice last i src.str :: acc

                    else
                        acc

                acc2 =
                    String.slice (i + 1) (i + 1 + w) src.str :: acc1

                nextI =
                    i + 1 + w
            in
            unescapeAnyPunctLoop src nextI nextI acc2

        else
            unescapeAnyPunctLoop src last (i + 1) acc

    else
        unescapeAnyPunctLoop src last (i + charWidth (at i src.codes)) acc


{-| Replaces `/\\([\[\]])/g` with `$1` (`rules.other.outputLinkReplace`).
-}
unescapeLinkBrackets : String -> String
unescapeLinkBrackets s =
    if not (String.contains "\\" s) then
        s

    else
        let
            src =
                fromString s
        in
        unescapeLinkBracketsLoop src 0 0 []
            |> List.reverse
            |> String.concat


unescapeLinkBracketsLoop : Src -> Int -> Int -> List String -> List String
unescapeLinkBracketsLoop src last i acc =
    if i + 1 - src.len >= 0 then
        if last - src.len < 0 then
            String.slice last src.len src.str :: acc

        else
            acc

    else if at i src.codes == 92 then
        let
            nextC =
                at (i + 1) src.codes
        in
        if nextC == 91 || nextC == 93 then
            let
                acc1 =
                    if i - last > 0 then
                        String.slice last i src.str :: acc

                    else
                        acc

                bracket =
                    if nextC == 91 then
                        "["

                    else
                        "]"
            in
            unescapeLinkBracketsLoop src (i + 2) (i + 2) (bracket :: acc1)

        else
            unescapeLinkBracketsLoop src last (i + 1) acc

    else
        unescapeLinkBracketsLoop src last (i + 1) acc


{-| Normalises `\r\n` and `\r` to `\n` (`other.carriageReturn`).
-}
normalizeNewlines : String -> String
normalizeNewlines src =
    if String.contains "\r" src then
        src
            |> String.replace "\r\n" "\n"
            |> String.replace "\r" "\n"

    else
        src
