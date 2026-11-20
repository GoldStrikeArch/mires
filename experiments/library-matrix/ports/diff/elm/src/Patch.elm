module Patch exposing (Patch, absent, apply, create, field, fields, format, parse, read, reverse, string, write)

{-| Unified patches after jsdiff 9.0.0 patch/create.js, parse.js, apply.js and
reverse.js.

A patch's metadata is an ordered list of fields, because the order in which
jsdiff first assigns each property is the order of the object it returns.
Hunk ranges are JavaScript numbers, so parsed headers keep values past int32
and NaN. Every loop over lines, hunks or files is a self tail call.

-}

import Array exposing (Array)
import Differ exposing (Kind(..))
import Json.Decode as D
import Json.Encode as E
import Utf16


type alias Fields =
    List ( String, E.Value )


type alias Hunk =
    { oldStart : Float, oldLines : Float, newStart : Float, newLines : Float, lines : List String }


type alias Patch =
    { meta : Fields, hunks : List Hunk }


{-| JSON ports cannot carry undefined; the host turns this marker back into it.
-}
absent : E.Value
absent =
    E.object [ ( "$diffUndefined", E.bool True ) ]


field : String -> D.Value -> E.Value
field k v =
    D.decodeValue (D.field k D.value) v |> Result.withDefault absent


fields : D.Value -> Fields
fields v =
    D.decodeValue (D.keyValuePairs D.value) v |> Result.withDefault []


val : String -> Fields -> E.Value
val k meta =
    List.filter (Tuple.first >> (==) k) meta |> List.head |> Maybe.map Tuple.second |> Maybe.withDefault absent


has : String -> Fields -> Bool
has k meta =
    List.any (Tuple.first >> (==) k) meta


string : D.Value -> String
string v =
    D.decodeValue D.string v |> Result.withDefault ""


flag : String -> D.Value -> Bool
flag k v =
    D.decodeValue (D.field k D.bool) v |> Result.withDefault False


{-| Assigns a property: an existing key keeps its position.
-}
put : String -> E.Value -> Fields -> Fields
put k v meta =
    if has k meta then
        List.map
            (\( key, prev ) ->
                if key == k then
                    ( k, v )

                else
                    ( key, prev )
            )
            meta

    else
        meta ++ [ ( k, v ) ]


write : Patch -> E.Value
write p =
    let
        hunk h =
            E.object
                [ ( "oldStart", E.float h.oldStart )
                , ( "oldLines", E.float h.oldLines )
                , ( "newStart", E.float h.newStart )
                , ( "newLines", E.float h.newLines )
                , ( "lines", E.list E.string h.lines )
                ]
    in
    E.object (put "hunks" (E.list hunk p.hunks) p.meta)


read : D.Value -> Patch
read v =
    let
        number k x =
            D.decodeValue (D.field k D.float) x |> Result.withDefault (0 / 0)

        hunk x =
            Hunk (number "oldStart" x)
                (number "oldLines" x)
                (number "newStart" x)
                (number "newLines" x)
                (D.decodeValue (D.field "lines" (D.list D.string)) x |> Result.withDefault [])
    in
    Patch (fields v) (D.decodeValue (D.field "hunks" (D.list (D.map hunk D.value))) v |> Result.withDefault [])



-- STRUCTURED PATCH AND FORMAT


emit : Kind -> Array String -> Int -> Int -> Hunk -> Hunk
emit kind source start count h =
    let
        tag =
            case kind of
                Keep ->
                    " "

                Add ->
                    "+"

                Remove ->
                    "-"

        line s acc =
            if String.endsWith "\n" s then
                (tag ++ String.dropRight 1 s) :: acc

            else
                "\\ No newline at end of file" :: (tag ++ s) :: acc

        old =
            if kind == Add then
                0

            else
                count

        new =
            if kind == Remove then
                0

            else
                count
    in
    { h
        | oldLines = h.oldLines + toFloat old
        , newLines = h.newLines + toFloat new
        , lines = Array.foldl line h.lines (Array.slice start (start + count) source)
    }


create : D.Value -> Int -> Patch
create input ctx =
    let
        before =
            Array.fromList (Differ.lineTokens Differ.defaults (string (field "before" input)))

        after =
            Array.fromList (Differ.lineTokens Differ.defaults (string (field "after" input)))

        meta =
            List.map (\k -> ( k, field k input )) [ "oldFileName", "newFileName", "oldHeader", "newHeader" ]

        fresh =
            Hunk 0 0 0 0 []

        closed h =
            { h | lines = List.reverse h.lines }

        build spans a b kept pending rev =
            case spans of
                [] ->
                    List.reverse
                        (if pending.oldStart == 0 then
                            rev

                         else
                            closed pending :: rev
                        )

                r :: rest ->
                    if r.kind == Keep then
                        let
                            close =
                                List.isEmpty rest || r.count > ctx * 2

                            next =
                                if pending.oldStart == 0 then
                                    fresh

                                else
                                    emit Keep after b (if close then min r.count ctx else r.count) pending
                        in
                        if close && pending.oldStart /= 0 then
                            build rest (a + r.count) (b + r.count) r.count fresh (closed next :: rev)

                        else
                            build rest (a + r.count) (b + r.count) r.count next rev

                    else
                        let
                            k =
                                min ctx kept

                            opened =
                                if pending.oldStart == 0 then
                                    emit Keep after (b - k) k { fresh | oldStart = toFloat (a + 1 - k), newStart = toFloat (b + 1 - k) }

                                else
                                    pending
                        in
                        if r.kind == Add then
                            build rest a (b + r.count) kept (emit Add after b r.count opened) rev

                        else
                            build rest (a + r.count) b kept (emit Remove before a r.count opened) rev
    in
    Patch meta (build (Differ.myers before after False) 0 0 0 fresh [])


{-| TextEncoder's bytes for one character; an isolated surrogate is U+FFFD.
-}
utf8 : Char -> List Int
utf8 c =
    let
        part =
            String.fromChar c

        x =
            if String.length part == 1 && part >= String.fromChar (Char.fromCode 0xD800) && part <= String.fromChar (Char.fromCode 0xDFFF) then
                0xFFFD

            else
                Char.toCode c

        cont n =
            128 + modBy 64 n
    in
    if x < 128 then
        [ x ]

    else if x < 0x0800 then
        [ 192 + x // 64, cont x ]

    else if x < 0x00010000 then
        [ 224 + x // 4096, cont (x // 64), cont x ]

    else
        [ 240 + x // 262144, cont (x // 4096), cont (x // 64), cont x ]


{-| quoteFileNameIfNeeded
-}
quote : String -> String
quote name =
    let
        bytes =
            List.concatMap utf8 (Utf16.toList name)

        digit n =
            Char.fromCode (48 + n)

        esc n =
            if n >= 7 && n <= 13 then
                "\\" ++ String.slice (n - 7) (n - 6) "abtnvfr"

            else if n == 34 || n == 92 then
                "\\" ++ String.fromChar (Char.fromCode n)

            else if n < 32 || n > 126 then
                String.fromList [ '\\', digit (n // 64), digit (modBy 8 (n // 8)), digit (modBy 8 n) ]

            else
                String.fromChar (Char.fromCode n)
    in
    if List.any (\n -> n < 32 || n > 126 || n == 34 || n == 92) bytes then
        "\"" ++ String.concat (List.map esc bytes) ++ "\""

    else
        name


format : Patch -> String
format p =
    let
        old =
            val "oldFileName" p.meta

        new =
            val "newFileName" p.meta

        text v =
            string v

        header prefix side =
            let
                h =
                    text (val (side ++ "Header") p.meta)
            in
            prefix
                ++ quote (text (val (side ++ "FileName") p.meta))
                ++ (if h == "" then
                        ""

                    else
                        "\t" ++ h
                   )
                ++ "\n"

        -- The start printed for an empty range is one lower.
        range start count =
            String.fromFloat
                (if count == 0 then
                    start - 1

                 else
                    start
                )
                ++ ","
                ++ String.fromFloat count

        hunk h =
            "@@ -" ++ range h.oldStart h.oldLines ++ " +" ++ range h.newStart h.newLines ++ " @@\n" ++ String.concat (List.map (\l -> l ++ "\n") h.lines)
    in
    (if old /= absent && old == new then
        "Index: " ++ text old ++ "\n"

     else
        ""
    )
        ++ "===================================================================\n"
        ++ (if old /= absent && new /= absent then
                header "--- " "old" ++ header "+++ " "new"

            else
                ""
           )
        ++ String.concat (List.map hunk p.hunks)



-- LINE CLASSES OF parse.js
-- Its regexes are anchored at the start of a line. String.trim and
-- String.trimLeft are the host's, so `\s` is the ECMAScript whitespace set;
-- `\w` is ASCII. Indexes count UTF-16 units, as JavaScript's do.


underline : String
underline =
    "==================================================================="


unit : Int -> String -> String
unit i s =
    String.slice i (i + 1) s


spaceAt : Int -> String -> Bool
spaceAt i s =
    let
        u =
            unit i s
    in
    u /= "" && String.trim u == ""


spaceAfter : String -> String -> Bool
spaceAfter prefix line =
    String.startsWith prefix line && spaceAt (String.length prefix) line


{-| `/^diff --git /`
-}
isGitHeader : String -> Bool
isGitHeader =
    String.startsWith "diff --git "


{-| `/^(---|\+\+\+)\s/`
-}
isFileHeader : String -> Bool
isFileHeader line =
    spaceAfter "---" line || spaceAfter "+++" line


{-| `/^@@\s/`
-}
isHunkHeader : String -> Bool
isHunkHeader =
    spaceAfter "@@"


wordAt : Int -> String -> Bool
wordAt i s =
    let
        u =
            unit i s
    in
    u /= "" && String.all (\c -> Char.isAlphaNum c || c == '_') u


{-| The end of `diff(?: -r \w+)+` in a match of `/^diff(?: -r \w+)+\s/`. The
repetition is greedy; a last group not followed by whitespace is given back,
and the space that starts it matches `\s`.
-}
hgPrefix : String -> Maybe Int
hgPrefix line =
    let
        wordEnd j =
            if wordAt j line then
                wordEnd (j + 1)

            else
                j

        groups at ends =
            if String.slice at (at + 4) line == " -r " && wordAt (at + 4) line then
                let
                    stop =
                        wordEnd (at + 4)
                in
                groups stop (stop :: ends)

            else
                ends
    in
    if String.startsWith "diff" line then
        case groups 4 [] of
            last :: previous :: _ ->
                if spaceAt last line then
                    Just last

                else
                    Just previous

            [ last ] ->
                if spaceAt last line then
                    Just last

                else
                    Nothing

            [] ->
                Nothing

    else
        Nothing


{-| The length of the match of `/^(?:Index:|diff(?: -r \w+)+)\s+/`.
-}
indexHeader : String -> Maybe Int
indexHeader line =
    let
        start =
            if String.startsWith "Index:" line then
                Just 6

            else
                hgPrefix line

        spaces n =
            let
                rest =
                    String.dropLeft n line
            in
            String.length rest - String.length (String.trimLeft rest)
    in
    start
        |> Maybe.andThen
            (\n ->
                if spaceAt n line then
                    Just (n + spaces n)

                else
                    Nothing
            )


{-| A Git, `Index:` or Mercurial header.
-}
isDiffHeader : String -> Bool
isDiffHeader line =
    isGitHeader line || indexHeader line /= Nothing


{-| The capture of `(.*)`: `.` stops at \\r, U+2028 and U+2029.
-}
restOfLine : String -> String
restOfLine s =
    case List.minimum (List.concatMap (\t -> List.take 1 (String.indexes t s)) [ "\u{000D}", "\u{2028}", "\u{2029}" ]) of
        Just end ->
            String.left end s

        Nothing ->
            s


digitCount : String -> Int
digitCount s =
    let
        count i =
            if String.all Char.isDigit (unit i s) && unit i s /= "" then
                count (i + 1)

            else
                i
    in
    count 0


{-| The capture of `(\d+)`, ASCII digits only.
-}
mode : String -> Maybe String
mode s =
    case digitCount s of
        0 ->
            Nothing

        n ->
            Just (String.left n s)



-- QUOTED NAMES


type alias Utf8 =
    { need : Int, point : Int, low : Int, high : Int }


{-| TextDecoder: UTF-8 with each maximal ill-formed subpart replaced by U+FFFD,
and a leading BOM consumed.
-}
decodeUtf8 : List Int -> String
decodeUtf8 bytes =
    let
        start =
            Utf8 0 0 0x80 0xBF

        expect need point low high =
            Utf8 need point low high

        decode input state acc =
            case input of
                [] ->
                    List.reverse
                        (if state.need > 0 then
                            '\u{FFFD}' :: acc

                         else
                            acc
                        )

                b :: rest ->
                    if state.need > 0 then
                        if b < state.low || b > state.high then
                            decode input start ('\u{FFFD}' :: acc)

                        else if state.need == 1 then
                            decode rest start (Char.fromCode (state.point * 64 + modBy 64 b) :: acc)

                        else
                            decode rest (expect (state.need - 1) (state.point * 64 + modBy 64 b) 0x80 0xBF) acc

                    else if b < 0x80 then
                        decode rest start (Char.fromCode b :: acc)

                    else if b >= 0xC2 && b <= 0xDF then
                        decode rest (expect 1 (b - 0xC0) 0x80 0xBF) acc

                    else if b >= 0xE0 && b <= 0xEF then
                        decode rest
                            (expect 2
                                (b - 0xE0)
                                (if b == 0xE0 then
                                    0xA0

                                 else
                                    0x80
                                )
                                (if b == 0xED then
                                    0x9F

                                 else
                                    0xBF
                                )
                            )
                            acc

                    else if b >= 0xF0 && b <= 0xF4 then
                        decode rest
                            (expect 3
                                (b - 0xF0)
                                (if b == 0xF0 then
                                    0x90

                                 else
                                    0x80
                                )
                                (if b == 0xF4 then
                                    0x8F

                                 else
                                    0xBF
                                )
                            )
                            acc

                    else
                        decode rest start ('\u{FFFD}' :: acc)
    in
    case decode bytes start [] of
        '\u{FEFF}' :: rest ->
            String.fromList rest

        points ->
            String.fromList points


escape : String -> Maybe String
escape u =
    case u of
        "a" ->
            Just "\u{0007}"

        "b" ->
            Just "\u{0008}"

        "f" ->
            Just "\u{000C}"

        "n" ->
            Just "\n"

        "r" ->
            Just "\u{000D}"

        "t" ->
            Just "\t"

        "v" ->
            Just "\u{000B}"

        "\\" ->
            Just "\\"

        "\"" ->
            Just "\""

        _ ->
            Nothing


{-| parseQuotedFileName: the unescaped name and the length of the quoted
text, or Nothing for an unterminated quote or an unknown escape. A run of
three-digit octal escapes holds raw bytes (Uint8Array keeps the low byte),
decoded together; `octets` is the pending run, latest byte first.
-}
quoted : String -> Maybe ( String, Int )
quoted s =
    let
        n =
            String.length s

        octalDigit j =
            if List.member (unit j s) [ "0", "1", "2", "3", "4", "5", "6", "7" ] then
                String.toInt (unit j s)

            else
                Nothing

        octal j =
            Maybe.map3 (\a b c -> modBy 256 (a * 64 + b * 8 + c)) (octalDigit j) (octalDigit (j + 1)) (octalDigit (j + 2))

        flush octets parts =
            if List.isEmpty octets then
                parts

            else
                decodeUtf8 (List.reverse octets) :: parts

        scan j octets parts =
            if j >= n then
                Nothing

            else
                let
                    u =
                        unit j s
                in
                if u == "\\" && octalDigit (j + 1) /= Nothing then
                    case octal (j + 1) of
                        Just byte ->
                            scan (j + 4) (byte :: octets) parts

                        Nothing ->
                            Nothing

                else if u == "\"" then
                    Just ( String.concat (List.reverse (flush octets parts)), j + 1 )

                else if u == "\\" && j + 1 < n then
                    case escape (unit (j + 1) s) of
                        Just c ->
                            scan (j + 2) [] (c :: flush octets parts)

                        Nothing ->
                            Nothing

                else
                    scan (j + 1) [] (u :: flush octets parts)
    in
    if String.startsWith "\"" s then
        scan 1 [] []

    else
        Nothing


{-| unquoteIfQuoted
-}
unquote : String -> String
unquote s =
    quoted s |> Maybe.map Tuple.first |> Maybe.withDefault s


{-| parseGitDiffHeader: the names after `diff --git `, when they can be
split. Both quoted forms skip the one UTF-16 unit assumed to be a space.
-}
gitNames : String -> Maybe ( String, String )
gitNames rest =
    case String.indexes "\"" rest of
        0 :: _ ->
            quoted rest
                |> Maybe.andThen
                    (\( old, length ) ->
                        let
                            after =
                                String.dropLeft (length + 1) rest
                        in
                        if String.startsWith "\"" after then
                            quoted after |> Maybe.map (\( new, _ ) -> ( old, new ))

                        else
                            Just ( old, after )
                    )

        q :: _ ->
            quoted (String.dropLeft q rest) |> Maybe.map (\( new, _ ) -> ( String.left (q - 1) rest, new ))

        [] ->
            let
                -- Split on the middle " b/", which is right when both names agree.
                splits =
                    String.indexes " b/" rest
            in
            if String.startsWith "a/" rest then
                List.drop (List.length splits // 2) splits
                    |> List.head
                    |> Maybe.map (\mid -> ( String.left mid rest, String.dropLeft (mid + 1) rest ))

            else
                Nothing



-- PARSE


{-| One Git extended header line; at most one of parse.js's regexes matches.
-}
extendedHeader : String -> Fields -> Fields
extendedHeader line meta =
    let
        after tag =
            String.dropLeft (String.length tag) line

        named key prefix tag flagKey =
            meta |> put key (E.string (prefix ++ unquote (restOfLine (after tag)))) |> put flagKey (E.bool True)

        withMode key tag flags =
            case mode (after tag) of
                Just m ->
                    List.foldl (\f acc -> put f (E.bool True) acc) meta flags |> put key (E.string m)

                Nothing ->
                    meta
    in
    if String.startsWith "rename from " line then
        named "oldFileName" "a/" "rename from " "isRename"

    else if String.startsWith "rename to " line then
        named "newFileName" "b/" "rename to " "isRename"

    else if String.startsWith "copy from " line then
        named "oldFileName" "a/" "copy from " "isCopy"

    else if String.startsWith "copy to " line then
        named "newFileName" "b/" "copy to " "isCopy"

    else if String.startsWith "new file mode " line then
        withMode "newMode" "new file mode " [ "isCreate" ]

    else if String.startsWith "deleted file mode " line then
        withMode "oldMode" "deleted file mode " [ "isDelete" ]

    else if String.startsWith "old mode " line then
        withMode "oldMode" "old mode " []

    else if String.startsWith "new mode " line then
        withMode "newMode" "new mode " []

    else if String.startsWith "Binary files " line then
        put "isBinary" (E.bool True) meta

    else
        meta


extendedHeaders : List String -> Fields -> ( Fields, List String )
extendedHeaders lines meta =
    case lines of
        line :: rest ->
            if isFileHeader line || isHunkHeader line || isDiffHeader line then
                ( meta, lines )

            else
                extendedHeaders rest (extendedHeader line meta)

        [] ->
            ( meta, [] )


{-| The metadata loop of parseIndex. The flag is set when a second diff header
ends the file before its file headers.
-}
metadata : List String -> Bool -> Fields -> ( Fields, List String, Bool )
metadata lines seen meta =
    case lines of
        [] ->
            ( meta, [], False )

        line :: rest ->
            if isFileHeader line || isHunkHeader line then
                ( meta, lines, False )

            else if isGitHeader line then
                if seen then
                    ( meta, lines, True )

                else
                    let
                        named =
                            case gitNames (String.dropLeft 11 line) of
                                Just ( old, new ) ->
                                    put "isGit" (E.bool True) meta |> put "oldFileName" (E.string old) |> put "newFileName" (E.string new)

                                Nothing ->
                                    put "isGit" (E.bool True) meta

                        ( extended, remaining ) =
                            extendedHeaders rest named
                    in
                    metadata remaining True extended

            else
                case indexHeader line of
                    Just length ->
                        if seen then
                            ( meta, lines, True )

                        else
                            metadata rest True (put "index" (E.string (String.trim (String.dropLeft length line))) meta)

                    Nothing ->
                        metadata rest seen meta


{-| parseFileHeader: a `---` or `+++` line, if there is one.
-}
fileHeader : List String -> Fields -> ( Fields, List String )
fileHeader lines meta =
    case lines of
        line :: rest ->
            if isFileHeader line then
                let
                    data =
                        String.split "\t" (String.trim (String.dropLeft 3 line))

                    name =
                        List.head data |> Maybe.withDefault ""

                    header =
                        List.head (List.drop 1 data) |> Maybe.withDefault "" |> String.trim

                    path =
                        if String.startsWith "\"" name then
                            unquote name

                        else
                            String.replace "\\\\" "\\" name

                    side =
                        if String.startsWith "---" line then
                            "old"

                        else
                            "new"
                in
                ( meta |> put (side ++ "FileName") (E.string path) |> put (side ++ "Header") (E.string header), rest )

            else
                ( meta, lines )

        [] ->
            ( meta, [] )


{-| A run of ASCII digits as a JavaScript number; a huge run is infinity.
-}
digits : String -> Maybe ( Float, String )
digits s =
    case digitCount s of
        0 ->
            Nothing

        n ->
            String.toFloat (String.left n s) |> Maybe.map (\x -> ( x, String.dropLeft n s ))


optionalCount : String -> ( Maybe Float, String )
optionalCount s =
    case
        if String.startsWith "," s then
            digits (String.dropLeft 1 s)

        else
            Nothing
    of
        Just ( n, rest ) ->
            ( Just n, rest )

        Nothing ->
            ( Nothing, s )


type alias Range =
    { oldStart : Float, oldLines : Maybe Float, newStart : Float, newLines : Maybe Float }


{-| The captures of `/@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/` at the
start of `s`, just after its leading `@@ -`.
-}
rangeAt : String -> Maybe Range
rangeAt s =
    digits s
        |> Maybe.andThen
            (\( oldStart, a ) ->
                let
                    ( oldLines, b ) =
                        optionalCount a
                in
                if String.startsWith " +" b then
                    digits (String.dropLeft 2 b)
                        |> Maybe.andThen
                            (\( newStart, c ) ->
                                let
                                    ( newLines, d ) =
                                        optionalCount c
                                in
                                if String.startsWith " @@" d then
                                    Just (Range oldStart oldLines newStart newLines)

                                else
                                    Nothing
                            )

                else
                    Nothing
            )


{-| The leftmost match, which String.prototype.split finds anywhere in the line.
-}
hunkRange : String -> Maybe Range
hunkRange line =
    let
        firstMatch starts =
            case starts of
                at :: more ->
                    case rangeAt (String.dropLeft (at + 4) line) of
                        Just range ->
                            Just range

                        Nothing ->
                            firstMatch more

                [] ->
                    Nothing
    in
    firstMatch (String.indexes "@@ -" line)


{-| parseHunk: an unmatched header gives NaN starts and one line per side.
-}
parseHunk : String -> List String -> Result String ( Hunk, List String )
parseHunk header body =
    let
        range =
            hunkRange header |> Maybe.withDefault (Range (0 / 0) Nothing (0 / 0) Nothing)

        oldLines =
            Maybe.withDefault 1 range.oldLines

        newLines =
            Maybe.withDefault 1 range.newLines

        h =
            { oldStart =
                if oldLines == 0 then
                    range.oldStart + 1

                else
                    range.oldStart
            , oldLines = oldLines
            , newStart =
                if newLines == 0 then
                    range.newStart + 1

                else
                    range.newStart
            , newLines = newLines
            , lines = []
            }

        finish rest adds removes taken =
            let
                newCount =
                    if adds == 0 && h.newLines == 1 then
                        0

                    else
                        h.newLines

                oldCount =
                    if removes == 0 && h.oldLines == 1 then
                        0

                    else
                        h.oldLines

                extra =
                    case rest of
                        following :: _ ->
                            following /= "" && List.member (String.left 1 following) [ "+", " ", "-" ] && not (isFileHeader following)

                        [] ->
                            False
            in
            if toFloat adds /= newCount then
                Err "added line count did not match"

            else if toFloat removes /= oldCount then
                Err "removed line count did not match"

            else if extra then
                Err "hunk has more lines than expected"

            else
                Ok ( { h | oldLines = oldCount, newLines = newCount, lines = List.reverse taken }, rest )

        consume lines adds removes taken =
            case lines of
                line :: rest ->
                    if toFloat removes < h.oldLines || toFloat adds < h.newLines || String.startsWith "\\" line then
                        -- An empty line is context, except the last one: ''[0] is undefined.
                        case
                            if line == "" && not (List.isEmpty rest) then
                                " "

                            else
                                String.left 1 line
                        of
                            "+" ->
                                consume rest (adds + 1) removes (line :: taken)

                            "-" ->
                                consume rest adds (removes + 1) (line :: taken)

                            " " ->
                                consume rest (adds + 1) (removes + 1) (line :: taken)

                            "\\" ->
                                consume rest adds removes (line :: taken)

                            _ ->
                                Err "hunk contained an invalid line"

                    else
                        finish lines adds removes taken

                [] ->
                    finish [] adds removes taken
    in
    consume body 0 0 []


hunks : List String -> List Hunk -> Result String ( List Hunk, List String )
hunks lines done =
    case lines of
        [] ->
            Ok ( List.reverse done, [] )

        line :: rest ->
            if isDiffHeader line || isFileHeader line || String.startsWith underline line then
                Ok ( List.reverse done, lines )

            else if isHunkHeader line then
                case parseHunk line rest of
                    Ok ( hunk, remaining ) ->
                        hunks remaining (hunk :: done)

                    Err e ->
                        Err e

            else
                hunks rest done


{-| parseIndex: one file's metadata, file headers and hunks.
-}
parseIndex : List String -> Result String ( Patch, List String )
parseIndex lines =
    let
        ( meta, afterMeta, endedEarly ) =
            metadata lines False [ ( "hunks", absent ) ]

        ( withOld, afterOld ) =
            fileHeader afterMeta meta

        ( named, afterNames ) =
            fileHeader afterOld withOld
    in
    if endedEarly then
        Ok ( Patch meta [], afterMeta )

    else if has "oldFileName" named /= has "newFileName" named then
        Err "missing file header"

    else
        hunks afterNames [] |> Result.map (\( hs, rest ) -> ( Patch named hs, rest ))


parse : String -> Result String (List Patch)
parse text =
    let
        files lines done =
            case lines of
                [] ->
                    Ok (List.reverse done)

                _ ->
                    case parseIndex lines of
                        Ok ( p, rest ) ->
                            files rest (p :: done)

                        Err e ->
                            Err e
    in
    files (String.split "\n" text) []



-- REVERSE
-- Over the JSON values, as Object.assign copies them.


reverse : D.Value -> E.Value
reverse v =
    let
        swapPrefix k =
            let
                value =
                    field k v
            in
            if flag "isGit" v then
                case D.decodeValue (D.field k D.string) v of
                    Ok name ->
                        if String.startsWith "a/" name then
                            E.string ("b/" ++ String.dropLeft 2 name)

                        else if String.startsWith "b/" name then
                            E.string ("a/" ++ String.dropLeft 2 name)

                        else
                            value

                    Err _ ->
                        value

            else
                value

        line l =
            if String.startsWith "-" l then
                "+" ++ String.dropLeft 1 l

            else if String.startsWith "+" l then
                "-" ++ String.dropLeft 1 l

            else
                l

        hunk h =
            E.object
                [ ( "oldLines", field "newLines" h )
                , ( "oldStart", field "newStart" h )
                , ( "newLines", field "oldLines" h )
                , ( "newStart", field "oldStart" h )
                , ( "lines", E.list (line >> E.string) (D.decodeValue (D.field "lines" (D.list D.string)) h |> Result.withDefault []) )
                ]

        swapped =
            fields v
                |> put "oldFileName" (swapPrefix "newFileName")
                |> put "oldHeader" (field "newHeader" v)
                |> put "newFileName" (swapPrefix "oldFileName")
                |> put "newHeader" (field "oldHeader" v)
                |> put "oldMode" (field "newMode" v)
                |> put "newMode" (field "oldMode" v)
                |> put "isCreate" (field "isDelete" v)
                |> put "isDelete" (field "isCreate" v)
                |> put "hunks" (E.list hunk (D.decodeValue (D.field "hunks" (D.list D.value)) v |> Result.withDefault []))
    in
    if flag "isCopy" v then
        -- Reversing a copy deletes the copy destination.
        swapped
            |> put "newFileName" (E.string "/dev/null")
            |> put "newHeader" absent
            |> put "isDelete" (E.bool True)
            |> List.filter (\( k, _ ) -> not (List.member k [ "isCreate", "isCopy", "isRename" ]))
            |> put "hunks" (E.list identity [])
            |> E.object

    else
        E.object swapped



-- APPLY


{-| hasOnlyWinLineEndings
-}
onlyWinLineEndings : String -> Bool
onlyWinLineEndings s =
    String.contains "\u{000D}\n" s
        && not (String.startsWith "\n" s)
        && not (List.any (\i -> i > 0 && unit (i - 1) s /= "\u{000D}") (String.indexes "\n" s))


{-| hasOnlyUnixLineEndings
-}
onlyUnixLineEndings : String -> Bool
onlyUnixLineEndings s =
    not (String.contains "\u{000D}\n" s) && String.contains "\n" s


{-| Each line with its successor ("" after the last line, which is not a marker).
-}
withNext : List String -> List ( String, String )
withNext lines =
    List.map2 Tuple.pair lines (List.drop 1 lines ++ [ "" ])


isUnix : List Hunk -> Bool
isUnix hs =
    not (List.any (\h -> List.any (\l -> not (String.startsWith "\\" l) && String.endsWith "\u{000D}" l) h.lines) hs)


isWin : List Hunk -> Bool
isWin hs =
    List.any (\h -> List.any (String.endsWith "\u{000D}") h.lines) hs
        && List.all (\h -> List.all (\( l, following ) -> String.startsWith "\\" l || String.endsWith "\u{000D}" l || String.startsWith "\\" following) (withNext h.lines)) hs


unixToWin : List Hunk -> List Hunk
unixToWin =
    List.map
        (\h ->
            { h
                | lines =
                    List.map
                        (\( l, following ) ->
                            if String.startsWith "\\" l || String.endsWith "\u{000D}" l || String.startsWith "\\" following then
                                l

                            else
                                l ++ "\u{000D}"
                        )
                        (withNext h.lines)
            }
        )


winToUnix : List Hunk -> List Hunk
winToUnix =
    List.map
        (\h ->
            { h
                | lines =
                    List.map
                        (\l ->
                            if String.endsWith "\u{000D}" l then
                                String.dropRight 1 l

                            else
                                l
                        )
                        h.lines
            }
        )


{-| distanceIterator: the positions after `start`, alternating forward and
backward by increasing distance, within `[min, max]`.
-}
type alias Distance =
    { start : Int, min : Int, max : Int, wantForward : Bool, backwardExhausted : Bool, forwardExhausted : Bool, offset : Int }


distance : Int -> Int -> Int -> Distance
distance start low high =
    Distance start low high True False False 1


step : Distance -> Maybe ( Int, Distance )
step d =
    let
        forward =
            if d.wantForward && not d.forwardExhausted then
                let
                    stepped =
                        if d.backwardExhausted then
                            { d | offset = d.offset + 1 }

                        else
                            { d | wantForward = False }
                in
                if stepped.start + stepped.offset <= stepped.max then
                    Ok ( stepped.start + stepped.offset, stepped )

                else
                    Err { stepped | forwardExhausted = True }

            else
                Err d
    in
    case forward of
        Ok found ->
            Just found

        Err e ->
            if not e.backwardExhausted then
                let
                    turned =
                        if e.forwardExhausted then
                            e

                        else
                            { e | wantForward = True }
                in
                if turned.min <= turned.start - turned.offset then
                    Just ( turned.start - turned.offset, { turned | offset = turned.offset + 1 } )

                else
                    step { turned | backwardExhausted = True }

            else
                Nothing


{-| applyHunk with no errors allowed: every removed and context line equals
the source line at its position. Trailing context is given back. The result
is the patched lines and the last old line consumed.
-}
fit : Array String -> List String -> Int -> Maybe ( List String, Int )
fit data hunkLines start =
    let
        go lines toPos patched context =
            case lines of
                [] ->
                    Just ( List.reverse (List.drop context patched), toPos - context - 1 )

                line :: rest ->
                    let
                        content =
                            String.dropLeft 1 line
                    in
                    case
                        if line == "" then
                            " "

                        else
                            String.left 1 line
                    of
                        "-" ->
                            if Array.get toPos data == Just content then
                                go rest (toPos + 1) patched 0

                            else
                                Nothing

                        "+" ->
                            go rest toPos (content :: patched) 0

                        " " ->
                            if Array.get toPos data == Just content then
                                go rest (toPos + 1) (content :: patched) (context + 1)

                            else
                                Nothing

                        _ ->
                            go rest toPos patched context
    in
    go hunkLines start [] 0


apply : String -> Patch -> Maybe String
apply source p =
    let
        hs =
            if onlyWinLineEndings source && isUnix p.hunks then
                unixToWin p.hunks

            else if onlyUnixLineEndings source && isWin p.hunks then
                winToUnix p.hunks

            else
                p.hunks

        raw =
            Array.fromList (String.split "\n" source)

        eofFlags remaining previous remove add =
            case remaining of
                [] ->
                    ( remove, add )

                l :: rest ->
                    if String.startsWith "\\" l then
                        eofFlags rest l (remove || String.startsWith "+" previous) (add || not (String.startsWith "+" previous) && String.startsWith "-" previous)

                    else
                        eofFlags rest l remove add

        ( removeEofnl, addEofnl ) =
            List.reverse hs |> List.head |> Maybe.map (\h -> eofFlags h.lines "" False False) |> Maybe.withDefault ( False, False )

        eof =
            Array.get (Array.length raw - 1) raw == Just ""

        lines =
            if removeEofnl then
                if addEofnl then
                    if eof then
                        Nothing

                    else
                        Just raw

                else if eof then
                    Just (Array.slice 0 -1 raw)

                else
                    Nothing

            else if addEofnl then
                if eof then
                    Nothing

                else
                    Just (Array.push "" raw)

            else
                Just raw
    in
    if List.isEmpty hs then
        Just source

    else
        Maybe.andThen (\data -> applyHunks data hs 0 0 []) lines


{-| Lines from `i` up to `stop`, latest first; a position outside the source
reads as undefined, which joins as an empty line.
-}
copy : Array String -> Int -> Int -> List String -> List String
copy data i stop acc =
    if i >= stop then
        acc

    else
        copy data (i + 1) stop ((Array.get i data |> Maybe.withDefault "") :: acc)


search : Array String -> Hunk -> Int -> Distance -> Maybe ( Int, ( List String, Int ) )
search data hunk toPos candidates =
    case fit data hunk.lines toPos of
        Just fitted ->
            Just ( toPos, fitted )

        Nothing ->
            case step candidates of
                Just ( position, rest ) ->
                    search data hunk position rest

                Nothing ->
                    Nothing


applyHunks : Array String -> List Hunk -> Int -> Int -> List String -> Maybe String
applyHunks data remaining minLine offset result =
    case remaining of
        [] ->
            Just (String.join "\n" (List.reverse (copy data minLine (Array.length data) result)))

        hunk :: rest ->
            let
                oldStart =
                    truncate hunk.oldStart

                -- The declared position first, then the distance iterator around it.
                toPos =
                    oldStart + offset - 1
            in
            case search data hunk toPos (distance toPos minLine (Array.length data - truncate hunk.oldLines)) of
                Nothing ->
                    Nothing

                Just ( at, ( patched, lastOldLine ) ) ->
                    applyHunks data rest (lastOldLine + 1) (at + 1 - oldStart) (List.foldl (::) (copy data minLine at result) patched)
