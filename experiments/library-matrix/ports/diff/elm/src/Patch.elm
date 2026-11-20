module Patch exposing (Patch, absent, apply, create, field, fields, format, object, parse, put, read, reverse, string, val, write)

import Utf16
import Array exposing (Array)
import Differ exposing (Kind(..))
import Json.Decode as D
import Json.Encode as E


type alias Fields =
    List ( String, E.Value )


type alias Hunk =
    { oldStart : Int, oldLines : Int, newStart : Int, newLines : Int, lines : List String }


type alias Patch =
    { meta : Fields, hunks : List Hunk }


absent =
    E.object [ ( "$diffUndefined", E.bool True ) ]


field k v =
    D.decodeValue (D.field k D.value) v |> Result.withDefault absent


fields v =
    D.decodeValue (D.keyValuePairs D.value) v |> Result.withDefault []


object =
    E.object


val k meta =
    List.filter (Tuple.first >> (==) k) meta |> List.head |> Maybe.map Tuple.second |> Maybe.withDefault absent


string v =
    D.decodeValue D.string v |> Result.withDefault ""


number v =
    D.decodeValue D.int v |> Result.withDefault 0


flag k m =
    D.decodeValue D.bool (val k m) |> Result.withDefault False


put k v meta =
    case meta of
        [] ->
            [ ( k, v ) ]

        ( key, prev ) :: xs ->
            if key == k then
                ( k, v ) :: xs

            else
                ( key, prev ) :: put k v xs


fresh =
    Hunk 0 0 0 0 []


write reversed p =
    let
        hunk h =
            let
                pairs =
                    if reversed then
                        [ ( "oldLines", h.oldLines ), ( "oldStart", h.oldStart ), ( "newLines", h.newLines ), ( "newStart", h.newStart ) ]

                    else
                        [ ( "oldStart", h.oldStart ), ( "oldLines", h.oldLines ), ( "newStart", h.newStart ), ( "newLines", h.newLines ) ]
            in
            object (List.map (Tuple.mapSecond E.int) pairs ++ [ ( "lines", E.list E.string h.lines ) ])
    in
    object (put "hunks" (E.list hunk p.hunks) p.meta)


read v =
    let
        h x =
            Hunk (number (field "oldStart" x))
                (number (field "oldLines" x))
                (number (field "newStart" x))
                (number (field "newLines" x))
                (D.decodeValue (D.list D.string) (field "lines" x) |> Result.withDefault [])
    in
    Patch (fields v) (D.decodeValue (D.list (D.map h D.value)) (field "hunks" v) |> Result.withDefault [])


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
    in
    { h
        | oldLines =
            h.oldLines
                + (if kind == Add then
                    0

                   else
                    count
                  )
        , newLines =
            h.newLines
                + (if kind == Remove then
                    0

                   else
                    count
                  )
        , lines = Array.foldl line h.lines (Array.slice start (start + count) source)
    }


create input ctx =
    let
        before =
            Array.fromList (Differ.lineTokens Differ.defaults (string (field "before" input)))

        after =
            Array.fromList (Differ.lineTokens Differ.defaults (string (field "after" input)))

        meta =
            List.map (\k -> ( k, field k input )) [ "oldFileName", "newFileName", "oldHeader", "newHeader" ]

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
                                    emit Keep
                                        after
                                        b
                                        (if close then
                                            min r.count ctx

                                         else
                                            r.count
                                        )
                                        pending
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
                                    emit Keep after (b - k) k { fresh | oldStart = a + 1 - k, newStart = b + 1 - k }

                                else
                                    pending
                        in
                        if r.kind == Add then
                            build rest a (b + r.count) kept (emit Add after b r.count opened) rev

                        else
                            build rest (a + r.count) b kept (emit Remove before a r.count opened) rev
    in
    Patch meta (build (Differ.myers before after False) 0 0 0 fresh [])


utf8 c =
    let
        x =
            let
                part =
                    String.fromChar c
            in
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


decodeUtf8 bytes =
    let
        continuation n cp low high rest =
            case rest of
                b :: tail ->
                    if b < low || b > high then
                        '�' :: decodeUtf8 rest

                    else if n == 1 then
                        Char.fromCode (cp * 64 + modBy 64 b) :: decodeUtf8 tail

                    else
                        continuation (n - 1) (cp * 64 + modBy 64 b) 128 191 tail

                [] ->
                    [ '�' ]
    in
    case bytes of
        [] ->
            []

        b :: rest ->
            if b < 128 then
                Char.fromCode b :: decodeUtf8 rest

            else if b >= 194 && b < 224 then
                continuation 1 (b - 192) 128 191 rest

            else if b >= 224 && b < 240 then
                continuation 2 (b - 224) (if b == 224 then 160 else 128) (if b == 237 then 159 else 191) rest

            else if b >= 240 && b <= 244 then
                continuation 3 (b - 240) (if b == 240 then 144 else 128) (if b == 244 then 143 else 191) rest

            else
                '�' :: decodeUtf8 rest


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


quoted s =
    let
        oct c =
            Char.toCode c - 48

        valid n =
            n >= 0 && n <= 7

        flush bytes =
            let
                points =
                    decodeUtf8 (List.reverse bytes)
            in
            case points of
                '\u{FEFF}' :: tail ->
                    String.fromList tail

                _ ->
                    String.fromList points

        parseChars chars octets parts consumed =
            case chars of
                '"' :: _ ->
                    Just ( String.concat (List.reverse (flush octets :: parts)), consumed + 1 )

                '\\' :: c :: rest ->
                    if valid (oct c) then
                        case rest of
                            x :: y :: more ->
                                if valid (oct x) && valid (oct y) then
                                    parseChars more (modBy 256 (oct c * 64 + oct x * 8 + oct y) :: octets) parts (consumed + 4)

                                else
                                    Nothing

                            _ ->
                                Nothing

                    else
                        let
                            escaped =
                                if c == '\\' || c == '"' then
                                    Just c

                                else
                                    String.indexes (String.fromChar c) "abtnvfr" |> List.head |> Maybe.map (\i -> Char.fromCode (i + 7))
                        in
                        Maybe.andThen (\ch -> parseChars rest [] (String.fromChar ch :: flush octets :: parts) (consumed + 2)) escaped

                c :: rest ->
                    parseChars rest [] (String.fromChar c :: flush octets :: parts) (consumed + String.length (String.fromChar c))

                [] ->
                    Nothing
    in
    if String.startsWith "\"" s then
        parseChars (Utf16.toList (String.dropLeft 1 s)) [] [] 1

    else
        Nothing


unquote s =
    quoted s |> Maybe.map Tuple.first |> Maybe.withDefault s


format p =
    let
        old =
            val "oldFileName" p.meta

        new =
            val "newFileName" p.meta

        header prefix side =
            let
                name =
                    string (val (side ++ "FileName") p.meta)

                h =
                    string (val (side ++ "Header") p.meta)
            in
            prefix ++ quote name
                ++ (if h == "" then
                        ""

                    else
                        "\t" ++ h
                   )
                ++ "\n"

        range a n =
            String.fromInt
                (a
                    - (if n == 0 then
                        1

                       else
                        0
                      )
                )
                ++ ","
                ++ String.fromInt n

        hunk h =
            "@@ -" ++ range h.oldStart h.oldLines ++ " +" ++ range h.newStart h.newLines ++ " @@\n" ++ String.concat (List.map (\l -> l ++ "\n") h.lines)
    in
    (if old /= absent && old == new then
        "Index: " ++ string old ++ "\n"

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


gitNames s meta =
    let
        names a b =
            meta |> put "oldFileName" (E.string a) |> put "newFileName" (E.string b)
    in
    case quoted s of
        Just ( a, n ) ->
            let
                b =
                    String.trim (String.dropLeft n s)
            in
            if b == "" then
                meta

            else
                names a (unquote b)

        Nothing ->
            case List.head (String.indexes "\"" s) of
                Just n ->
                    case quoted (String.dropLeft n s) of
                        Just ( b, _ ) ->
                            names (String.trimRight (String.left n s)) b

                        Nothing ->
                            meta

                Nothing ->
                    let
                        spots =
                            String.indexes " b/" s
                    in
                    case List.head (List.drop (List.length spots // 2) spots) of
                        Just n ->
                            if String.startsWith "a/" s then
                                names (String.left n s) (String.dropLeft (n + 1) s)

                            else
                                meta

                        Nothing ->
                            meta


extended l meta =
    let
        options =
            [ ( "rename from ", ( "oldFileName", "a/", "isRename" ) )
            , ( "rename to ", ( "newFileName", "b/", "isRename" ) )
            , ( "copy from ", ( "oldFileName", "a/", "isCopy" ) )
            , ( "copy to ", ( "newFileName", "b/", "isCopy" ) )
            ]

        opts =
            [ ( "new file mode ", ( "isCreate", "newMode" ) ), ( "deleted file mode ", ( "isDelete", "oldMode" ) ), ( "old mode ", ( "", "oldMode" ) ), ( "new mode ", ( "", "newMode" ) ) ]

        find xs =
            List.filter (\( prefix, _ ) -> String.startsWith prefix l) xs |> List.head
    in
    case find options of
        Just ( tag, ( k, p, f ) ) ->
            meta |> put k (E.string (p ++ unquote (String.dropLeft (String.length tag) l))) |> put f (E.bool True)

        Nothing ->
            case find opts of
                Just ( tag, ( f, k ) ) ->
                    (if f == "" then
                        meta

                     else
                        put f (E.bool True) meta
                    )
                        |> put k (E.string (String.dropLeft (String.length tag) l))

                Nothing ->
                    if String.startsWith "Binary files " l then
                        put "isBinary" (E.bool True) meta

                    else
                        meta


file l =
    List.any (\p -> String.startsWith p l) [ "--- ", "+++ ", "---\t", "+++\t" ]


headerLine l =
    List.any (\p -> String.startsWith p l) [ "diff --git ", "Index: ", "diff -r " ]


fileName l m =
    let
        parts =
            String.split "\t" (String.trim (String.dropLeft 3 l))

        n =
            List.head parts |> Maybe.withDefault ""

        h =
            List.head (List.drop 1 parts) |> Maybe.withDefault "" |> String.trim

        side =
            if String.startsWith "---" l then
                "old"

            else
                "new"

        path =
            if String.startsWith "\"" n then
                unquote n

            else
                String.replace "\\\\" "\\" n
    in
    m |> put (side ++ "FileName") (E.string path) |> put (side ++ "Header") (E.string h)


parseRange sign s =
    let
        result a b =
            Maybe.map2
                (\start count ->
                    ( start
                        + (if count == 0 then
                            1

                           else
                            0
                          )
                    , count
                    )
                )
                (String.toInt a)
                (String.toInt b)
    in
    if not (String.startsWith sign s) then
        Nothing

    else
        case String.split "," (String.dropLeft 1 s) of
            [ a ] ->
                result a "1"

            [ a, b ] ->
                result a b

            _ ->
                Nothing


parseHunk header body =
    let
        consume lines old new h rev =
            let
                done tail =
                    let
                        oc =
                            if old == 0 && h.oldLines == 1 then
                                0

                            else
                                h.oldLines

                        nc =
                            if new == 0 && h.newLines == 1 then
                                0

                            else
                                h.newLines

                        next =
                            List.head tail |> Maybe.withDefault ""
                    in
                    if oc /= old || nc /= new || (next /= "" && String.contains (String.left 1 next) "+ -" && not (file next)) then
                        Err "hunk count"

                    else
                        Ok ( { h | oldLines = oc, newLines = nc, lines = List.reverse rev }, tail )
            in
            case lines of
                l :: rest ->
                    if old < h.oldLines || new < h.newLines || String.startsWith "\\" l then
                        let
                            mark =
                                if l == "" && not (List.isEmpty rest) then
                                    " "

                                else
                                    String.left 1 l
                        in
                        if not (List.member mark [ "+", "-", " ", "\\" ]) then
                            Err "invalid line"

                        else
                            consume rest
                                (old
                                    + (if mark == " " || mark == "-" then
                                        1

                                       else
                                        0
                                      )
                                )
                                (new
                                    + (if mark == " " || mark == "+" then
                                        1

                                       else
                                        0
                                      )
                                )
                                h
                                (l :: rev)

                    else
                        done lines

                [] ->
                    done []
    in
    case String.split " " header of
        "@@" :: a :: b :: "@@" :: _ ->
            case ( parseRange "-" a, parseRange "+" b ) of
                ( Just ( o, oc ), Just ( n, nc ) ) ->
                    consume body 0 0 (Hunk o oc n nc []) []

                _ ->
                    Err "range"

        _ ->
            Err "header"


parse s =
    let
        metadata lines seen m =
            case lines of
                l :: rest ->
                    if file l || String.startsWith "@@ " l || (seen && headerLine l) then
                        ( m, lines )

                    else if String.startsWith "diff --git " l then
                        metadata rest True (gitNames (String.dropLeft 11 l) (put "isGit" (E.bool True) m))

                    else if String.startsWith "Index: " l then
                        metadata rest True (put "index" (E.string (String.trim (String.dropLeft 7 l))) m)

                    else if headerLine l then
                        metadata rest True (put "index" (E.string (List.reverse (String.words l) |> List.head |> Maybe.withDefault "")) m)

                    else
                        metadata rest
                            seen
                            (if flag "isGit" m then
                                extended l m

                             else
                                m
                            )

                [] ->
                    ( m, [] )

        filenames n lines m =
            case lines of
                l :: rest ->
                    if n > 0 && file l then
                        filenames (n - 1) rest (fileName l m)

                    else
                        ( m, lines )

                [] ->
                    ( m, [] )

        hunks lines rev =
            case lines of
                l :: rest ->
                    if file l || headerLine l || String.startsWith "===================================================================" l then
                        Ok ( List.reverse rev, lines )

                    else if String.startsWith "@@ " l then
                        parseHunk l rest |> Result.andThen (\( h, tail ) -> hunks tail (h :: rev))

                    else
                        hunks rest rev

                [] ->
                    Ok ( List.reverse rev, [] )

        files lines rev =
            case lines of
                [] ->
                    Ok (List.reverse rev)

                _ ->
                    let
                        ( meta, more ) =
                            metadata lines False [ ( "hunks", absent ) ]

                        ( m, body ) =
                            filenames 2 more meta
                    in
                    if (val "oldFileName" m == absent) /= (val "newFileName" m == absent) then
                        Err "missing file header"

                    else
                        hunks body [] |> Result.andThen (\( hs, tail ) -> files tail (Patch m hs :: rev))
    in
    files (String.split "\n" s) []


reverse p =
    let
        copy =
            flag "isCopy" p.meta

        git =
            flag "isGit" p.meta

        swapped k other acc =
            let
                v =
                    val other p.meta

                s =
                    string v

                w =
                    if git && String.endsWith "FileName" k && (String.startsWith "a/" s || String.startsWith "b/" s) then
                        E.string
                            ((if String.startsWith "a/" s then
                                "b/"

                              else
                                "a/"
                             )
                                ++ String.dropLeft 2 s
                            )

                    else
                        v
            in
            if copy && k == "isCreate" then
                acc

            else
                put k w acc

        props =
            [ ( "oldFileName", "newFileName" )
            , ( "oldHeader", "newHeader" )
            , ( "newFileName", "oldFileName" )
            , ( "newHeader", "oldHeader" )
            , ( "oldMode", "newMode" )
            , ( "newMode", "oldMode" )
            , ( "isCreate", "isDelete" )
            , ( "isDelete", "isCreate" )
            ]

        original =
            if copy then
                List.filter (\( k, _ ) -> not (List.member k [ "isCreate", "isCopy", "isRename" ])) p.meta

            else
                p.meta

        meta =
            List.foldl (\( k, v ) acc -> swapped k v acc) original props

        flip l =
            if String.startsWith "+" l then
                "-" ++ String.dropLeft 1 l

            else if String.startsWith "-" l then
                "+" ++ String.dropLeft 1 l

            else
                l

        hunk h =
            Hunk h.newStart h.newLines h.oldStart h.oldLines (List.map flip h.lines)
    in
    if copy then
        Patch (meta |> put "newFileName" (E.string "/dev/null") |> put "newHeader" absent |> put "isDelete" (E.bool True)) []

    else
        Patch meta (List.map hunk p.hunks)


convert source hs =
    let
        win =
            String.contains "\u{000D}\n" source

        unix =
            String.contains "\n" (String.replace "\u{000D}\n" "" source)

        marked lines =
            case lines of
                [] ->
                    []

                x :: rest ->
                    ( x, List.head rest |> Maybe.map (String.startsWith "\\") |> Maybe.withDefault False ) :: marked rest

        samples =
            List.concatMap (\h -> marked h.lines) hs |> List.filter (\( l, _ ) -> not (String.startsWith "\\" l))

        pw =
            List.any (\( l, _ ) -> String.endsWith "\u{000D}" l) samples

        pu =
            List.any (\( l, m ) -> not m && not (String.endsWith "\u{000D}" l)) samples

        convertLine ( l, m ) =
            if win && not pw && not m && not (String.startsWith "\\" l) then
                l ++ "\u{000D}"

            else if unix && pw && not pu && String.endsWith "\u{000D}" l then
                String.dropRight 1 l

            else
                l
    in
    if win == unix then
        hs

    else
        List.map (\h -> { h | lines = List.map convertLine (marked h.lines) }) hs


apply source p =
    let
        hs =
            convert source p.hunks

        eofFlags lines prev add remove =
            case lines of
                [] ->
                    ( add, remove )

                l :: rest ->
                    eofFlags rest (String.left 1 l) (add || (String.startsWith "\\" l && prev == "-")) (remove || (String.startsWith "\\" l && prev == "+"))

        ( ae, re ) =
            List.reverse hs |> List.head |> Maybe.map (\h -> eofFlags h.lines "" False False) |> Maybe.withDefault ( False, False )

        raw =
            Array.fromList (String.split "\n" source)

        eof =
            Array.get (Array.length raw - 1) raw == Just ""

        data =
            if re && not ae then
                Array.slice 0 -1 raw

            else if ae && not re then
                Array.push "" raw

            else
                raw

        fits lines i =
            case lines of
                [] ->
                    True

                l :: rest ->
                    if String.startsWith "+" l || String.startsWith "\\" l then
                        fits rest i

                    else if Array.get i data == Just (String.dropLeft 1 l) then
                        fits rest (i + 1)

                    else
                        False

        matchHunk h low offset =
            let
                high =
                    Array.length data - h.oldLines

                first =
                    clamp low high (h.oldStart + offset - 1)

                scan d =
                    if d > max (first - low) (high - first) then
                        Nothing

                    else if first + d <= high && fits h.lines (first + d) then
                        Just (first + d)

                    else if d > 0 && first - d >= low && fits h.lines (first - d) then
                        Just (first - d)

                    else
                        scan (d + 1)
            in
            if high < low then
                Nothing

            else
                scan 0

        copy a b rev =
            Array.foldl (::) rev (Array.slice a b data)

        patchLines lines cursor context rev =
            case lines of
                [] ->
                    ( cursor - context, List.drop context rev )

                l :: rest ->
                    if String.startsWith "+" l then
                        patchLines rest cursor 0 (String.dropLeft 1 l :: rev)

                    else if String.startsWith "-" l then
                        patchLines rest (cursor + 1) 0 rev

                    else if String.startsWith "\\" l then
                        patchLines rest cursor context rev

                    else
                        patchLines rest (cursor + 1) (context + 1) ((Array.get cursor data |> Maybe.withDefault "") :: rev)

        fit remaining low offset rev =
            case remaining of
                [] ->
                    Just (String.join "\n" (List.reverse (copy low (Array.length data) rev)))

                h :: rest ->
                    case matchHunk h low offset of
                        Nothing ->
                            Nothing

                        Just pos ->
                            let
                                ( next, patched ) =
                                    patchLines h.lines pos 0 (copy low pos rev)
                            in
                            fit rest next (pos + 1 - h.oldStart) patched
    in
    if List.isEmpty hs then
        Just source

    else if (re && ae && eof) || (re && not ae && not eof) || (not re && ae && eof) then
        Nothing

    else
        fit hs 0 0 []
