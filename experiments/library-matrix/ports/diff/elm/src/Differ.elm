module Differ exposing (Change, Kind(..), Options, Run, changes, defaults, lineTokens, myers, tokenize)

import Utf16
import Array exposing (Array)
import Dict


type Kind
    = Keep
    | Remove
    | Add


type alias Run =
    { kind : Kind, count : Int }


type alias Path =
    { old : Int, history : List Run }


type alias Change =
    { kind : Kind, count : Int, value : String }


type alias Options =
    { ignoreCase : Bool, individual : Bool, stripCr : Bool, ignoreWs : Bool, separateNl : Bool, ignoreEof : Bool }


defaults : Options
defaults =
    Options False False False False False False


addRun separate kind n history =
    if n == 0 then
        history

    else if separate then
        List.repeat n (Run kind 1) ++ history

    else
        case history of
            h :: tail ->
                if h.kind == kind then
                    Run kind (h.count + n) :: tail

                else
                    Run kind n :: history

            [] ->
                [ Run kind n ]


myers : Array String -> Array String -> Bool -> List Run
myers old new individual =
    let
        oldSize =
            Array.length old

        newSize =
            Array.length new

        limit =
            oldSize + newSize

        advance k path =
            let
                scan i j =
                    if i + 1 < oldSize && j + 1 < newSize && Array.get (i + 1) old == Array.get (j + 1) new then
                        scan (i + 1) (j + 1)

                    else
                        ( Path i (addRun individual Keep (i - path.old) path.history), j )
            in
            scan path.old (path.old - k)

        diagonal d k stop low high frontier =
            if d > limit then
                []

            else if k > stop then
                diagonal (d + 1) (max low -(d + 1)) (min high (d + 1)) low high frontier

            else
                let
                    rem =
                        Dict.get (k - 1) frontier |> Maybe.withDefault (Path -2 [])

                    ins =
                        Dict.get (k + 1) frontier |> Maybe.withDefault (Path -2 [])

                    r =
                        rem.old >= -1 && rem.old + 1 < oldSize

                    a =
                        ins.old >= -1 && ins.old - k >= 0 && ins.old - k < newSize

                    pruned =
                        Dict.remove (k - 1) frontier
                in
                if not (a || r) then
                    diagonal d (k + 2) stop low high (Dict.remove k pruned)

                else
                    let
                        next =
                            if not r || (a && rem.old < ins.old) then
                                Path ins.old (addRun individual Add 1 ins.history)

                            else
                                Path (rem.old + 1) (addRun individual Remove 1 rem.history)

                        ( path, j ) =
                            advance k next

                        lowNext =
                            if j + 1 == newSize then
                                max low (k + 1)

                            else
                                low

                        highNext =
                            if path.old + 1 == oldSize then
                                min high (k - 1)

                            else
                                high
                    in
                    if path.old + 1 == oldSize && j + 1 == newSize then
                        List.reverse path.history

                    else
                        diagonal d (k + 2) stop lowNext highNext (Dict.insert k path pruned)

        ( first, newPos ) =
            advance 0 (Path -1 [])
    in
    if first.old + 1 == oldSize && newPos + 1 == newSize then
        List.reverse first.history

    else
        diagonal 1 -1 1 -limit limit (Dict.singleton 0 first)


space c =
    String.trim (String.fromChar c) == ""


word c =
    let
        n =
            Char.toCode c

        between a b =
            a <= n && n <= b
    in
    Char.isAlphaNum c || c == '_' || n == 0xAD || between 0xC0 0xD6 || between 0xD8 0xF6 || between 0xF8 0x02C6 || between 0x02C8 0x02D7 || between 0x02DE 0x02FF || between 0x1E00 0x1EFF


words s =
    let
        collect cls rest rev =
            case rest of
                [] ->
                    ( String.fromList (List.reverse rev), [] )

                c :: more ->
                    if cls c then
                        collect cls more (c :: rev)

                    else
                        ( String.fromList (List.reverse rev), rest )

        scan input previous out =
            case input of
                [] ->
                    List.reverse out

                c :: tail ->
                    if space c then
                        let
                            ( p, rest ) =
                                collect space tail [ c ]

                            updated =
                                case out of
                                    [] ->
                                        [ p ]

                                    h :: t ->
                                        (h ++ p) :: t
                        in
                        scan rest p updated

                    else
                        let
                            ( p, rest ) =
                                if word c then
                                    collect word tail [ c ]

                                else
                                    ( String.fromChar c, tail )

                            updated =
                                case out of
                                    [ h ] ->
                                        if previous /= "" && h == previous then
                                            [ h ++ p ]

                                        else
                                            (previous ++ p) :: out

                                    _ ->
                                        (previous ++ p) :: out
                        in
                        scan rest "" updated
    in
    scan (Utf16.toList s) "" []


lineTokens o s =
    let
        group parts out =
            case parts of
                x :: y :: ys ->
                    let
                        cr =
                            String.endsWith "\u{000D}" x

                        content =
                            if cr then
                                String.dropRight 1 x

                            else
                                x

                        delimiter =
                            if cr then
                                "\u{000D}\n"

                            else
                                "\n"

                        next =
                            if o.separateNl then
                                if content == "" then
                                    delimiter :: out

                                else
                                    delimiter :: content :: out

                            else
                                (content ++ delimiter) :: out
                    in
                    group (y :: ys) next

                [ x ] ->
                    List.reverse
                        (if x == "" then
                            out

                         else
                            x :: out
                        )

                [] ->
                    List.reverse out
    in
    -- stripTrailingCr first replaces every \r\n with \n, as npm does, so in \r\r\n the first \r
    -- and the new \n make one separator.
    group
        (String.split "\n"
            (if o.stripCr then
                String.replace "\u{000D}\n" "\n" s

             else
                s
            )
        )
        []


tokenize op o s =
    if op == "diffWords" then
        words s

    else if op == "diffLines" || op == "diffJson" then
        lineTokens o s

    else
        List.map String.fromChar (Utf16.toList s)


normal op o original =
    let
        s =
            if op == "diffWords" || (op == "diffLines" && o.ignoreWs && (not o.separateNl || not (String.contains "\n" original))) then
                String.trim original

            else if op == "diffLines" && o.ignoreEof && not o.separateNl && String.endsWith "\n" original then
                String.dropRight 1 original

            else
                original

        t =
            if op == "diffJson" then
                String.replace ",\n" "\n" (String.replace ",\u{000D}" "\u{000D}" s)

            else
                s
    in
    if o.ignoreCase then
        String.toLower t

    else
        t


leading s =
    String.left (String.length s - String.length (String.trimLeft s)) s


trailing s =
    String.right (String.length s - String.length (String.trimRight s)) s


common a b =
    let
        consume x y =
            case ( x, y ) of
                ( c :: cs, d :: ds ) ->
                    if c == d then
                        c :: consume cs ds

                    else
                        []

                _ ->
                    []
    in
    consume (Utf16.toList a) (Utf16.toList b) |> String.fromList


suffix a b =
    String.reverse (common (String.reverse a) (String.reverse b))


overlap a b =
    let
        find n =
            if n == 0 || String.right n a == String.left n b then
                n

            else
                find (n - 1)
    in
    find (min (String.length a) (String.length b))


adjust start del ins end out =
    let
        value i =
            Array.get i out |> Maybe.map .value |> Maybe.withDefault ""

        update i newValue table =
            case Array.get i table of
                Just c ->
                    Array.set i { c | value = newValue } table

                Nothing ->
                    table

        d =
            value del

        a =
            value ins

        s =
            value start

        e =
            value end
    in
    if del >= 0 && ins >= 0 then
        let
            np =
                leading a

            ns =
                trailing a

            p =
                if start >= 0 then
                    common (leading d) np

                else
                    ""

            t =
                if end >= 0 then
                    suffix (trailing d) ns

                else
                    ""

            trimEnds v =
                String.dropRight (String.length t) (String.dropLeft (String.length p) v)
        in
        out
            |> update start (String.dropRight (String.length np) s ++ p)
            |> update end (t ++ String.dropLeft (String.length ns) e)
            |> update del (trimEnds d)
            |> update ins (trimEnds a)

    else if ins >= 0 then
        out
            |> update ins
                (if start >= 0 then
                    String.trimLeft a

                 else
                    a
                )
            |> update end (String.trimLeft e)

    else if del >= 0 && start >= 0 && end >= 0 then
        let
            w =
                leading e

            p =
                common w (leading d)

            t =
                suffix (String.dropLeft (String.length p) w) (trailing d)
        in
        out
            |> update del (String.dropRight (String.length t) (String.dropLeft (String.length p) d))
            |> update end (String.dropLeft (String.length w - String.length t) e)
            |> update start (String.dropRight (String.length t) s)

    else if del >= 0 && start >= 0 then
        update del (String.dropLeft (overlap (trailing s) (leading d)) d) out

    else if del >= 0 && end >= 0 then
        update del (String.dropRight (overlap (trailing d) (leading e)) d) out

    else
        out


changes op o before after =
    let
        old =
            Array.fromList before

        new =
            Array.fromList after

        spans =
            myers (Array.map (normal op o) old) (Array.map (normal op o) new) o.individual

        build rows a b rev =
            case rows of
                [] ->
                    List.reverse rev

                r :: rest ->
                    let
                        token j =
                            let
                                x =
                                    Array.get (a + j) old |> Maybe.withDefault ""

                                y =
                                    Array.get (b + j) new |> Maybe.withDefault ""

                                v =
                                    if r.kind == Remove || (op == "diffJson" && r.kind == Keep && String.length x > String.length y) then
                                        x

                                    else
                                        y
                            in
                            if op == "diffWords" && j > 0 then
                                String.trimLeft v

                            else
                                v

                        value =
                            List.range 0 (r.count - 1) |> List.map token |> String.concat
                    in
                    build rest
                        (a
                            + (if r.kind == Add then
                                0

                               else
                                r.count
                              )
                        )
                        (b
                            + (if r.kind == Remove then
                                0

                               else
                                r.count
                              )
                        )
                        (Change r.kind r.count value :: rev)

        clean i start del ins a =
            case Array.get i a of
                Nothing ->
                    adjust start del ins -1 a

                Just c ->
                    case c.kind of
                        Add ->
                            clean (i + 1) start del i a

                        Remove ->
                            clean (i + 1) start i ins a

                        Keep ->
                            clean (i + 1) i -1 -1 (adjust start del ins i a)

        raw =
            build spans 0 0 []
    in
    if op == "diffWords" && not o.individual then
        Array.toList (clean 0 -1 -1 -1 (Array.fromList raw))

    else
        raw
