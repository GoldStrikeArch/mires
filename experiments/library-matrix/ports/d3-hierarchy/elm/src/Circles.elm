module Circles exposing (layout)

import Arena exposing (alter, get)
import Array


type alias Circle =
    { x : Float, y : Float, r : Float }


origin =
    Circle 0 0 0


circle n =
    Circle n.x n.y n.r


weak a b =
    let
        dr =
            a.r - b.r + max (max a.r b.r) 1 * 1.0e-9

        dx =
            b.x - a.x

        dy =
            b.y - a.y
    in
    dr > 0 && dr * dr > dx * dx + dy * dy


outside a b =
    let
        dr =
            a.r - b.r

        dx =
            b.x - a.x

        dy =
            b.y - a.y
    in
    dr < 0 || dr * dr < dx * dx + dy * dy


pair a b =
    let
        dx =
            b.x - a.x

        dy =
            b.y - a.y

        dr =
            b.r - a.r

        d =
            sqrt (dx * dx + dy * dy)
    in
    Circle ((a.x + b.x + dx / d * dr) / 2) ((a.y + b.y + dy / d * dr) / 2) ((d + a.r + b.r) / 2)


triple a b c =
    let
        a2 =
            a.x - b.x

        a3 =
            a.x - c.x

        b2 =
            a.y - b.y

        b3 =
            a.y - c.y

        c2 =
            b.r - a.r

        c3 =
            c.r - a.r

        d1 =
            a.x * a.x + a.y * a.y - a.r * a.r

        d2 =
            d1 - b.x * b.x - b.y * b.y + b.r * b.r

        d3 =
            d1 - c.x * c.x - c.y * c.y + c.r * c.r

        ab =
            a3 * b2 - a2 * b3

        xa =
            (b2 * d3 - b3 * d2) / (ab * 2) - a.x

        xb =
            (b3 * c2 - b2 * c3) / ab

        ya =
            (a3 * d2 - a2 * d3) / (ab * 2) - a.y

        yb =
            (a2 * c3 - a3 * c2) / ab

        qa =
            xb * xb + yb * yb - 1

        qb =
            2 * (a.r + xa * xb + ya * yb)

        qc =
            xa * xa + ya * ya - a.r * a.r

        r =
            -(if abs qa > 1.0e-6 then
                (qb + sqrt (qb * qb - 4 * qa * qc)) / (2 * qa)

              else
                qc / qb
             )
    in
    Circle (a.x + xa + xb * r) (a.y + ya + yb * r) r


extend basis p =
    let
        covers a =
            List.all (weak a) basis

        pairs bs =
            case bs of
                [] ->
                    Nothing

                a :: tail ->
                    case List.filter (\b -> outside (pair a b) p && outside (pair a p) b && outside (pair b p) a && covers (triple a b p)) tail |> List.head of
                        Just b ->
                            Just ( [ a, b, p ], triple a b p )

                        Nothing ->
                            pairs tail
    in
    if covers p then
        Just ( [ p ], p )

    else
        case List.filter (\a -> outside p a && covers (pair a p)) basis |> List.head of
            Just a ->
                Just ( [ a, p ], pair a p )

            Nothing ->
                pairs basis


enclose circles seed =
    let
        shuffle arr n state =
            if n == 0 then
                ( Array.toList arr, state )

            else
                let
                    step =
                        1664525 * state + 1013904223

                    next =
                        step - toFloat (floor (step / 4294967296)) * 4294967296

                    i =
                        floor (next / 4294967296 * toFloat n)

                    a =
                        Maybe.withDefault origin (Array.get (n - 1) arr)

                    b =
                        Maybe.withDefault origin (Array.get i arr)
                in
                shuffle (arr |> Array.set (n - 1) b |> Array.set i a) (n - 1) next

        ( shuffled, newSeed ) =
            shuffle (Array.fromList circles) (List.length circles) seed

        scan rest basis result =
            case rest of
                [] ->
                    Ok ( result, newSeed )

                p :: tail ->
                    if not (List.isEmpty basis) && weak result p then
                        scan tail basis result

                    else
                        case extend basis p of
                            Just ( bs, e ) ->
                                scan shuffled bs e

                            Nothing ->
                                Err "invalid enclosing basis"
    in
    scan shuffled [] origin


place b a c =
    let
        dx =
            b.x - a.x

        dy =
            b.y - a.y

        d2 =
            dx * dx + dy * dy

        a2 =
            (a.r + c.r) * (a.r + c.r)

        b2 =
            (b.r + c.r) * (b.r + c.r)
    in
    if d2 == 0 then
        { c | x = a.x + c.r, y = a.y }

    else if a2 > b2 then
        let
            x =
                (d2 + b2 - a2) / (2 * d2)

            y =
                sqrt (max 0 (b2 / d2 - x * x))
        in
        { c | x = b.x - x * dx - y * dy, y = b.y - x * dy + y * dx }

    else
        let
            x =
                (d2 + a2 - b2) / (2 * d2)

            y =
                sqrt (max 0 (a2 / d2 - x * x))
        in
        { c | x = a.x + x * dx - y * dy, y = a.y + x * dy + y * dx }


intersects a b =
    let
        dr =
            a.r + b.r - 1.0e-6

        dx =
            b.x - a.x

        dy =
            b.y - a.y
    in
    dr > 0 && dr * dr > dx * dx + dy * dy


score a b =
    let
        ab =
            a.r + b.r

        dx =
            (a.x * b.r + b.x * a.r) / ab

        dy =
            (a.y * b.r + b.y * a.r) / ab
    in
    dx * dx + dy * dy


siblings list seed =
    let
        circles =
            Array.fromList list

        n =
            Array.length circles

        at arr i =
            Maybe.withDefault origin (Array.get i arr)

        index arr i =
            Maybe.withDefault 0 (Array.get i arr)

        first =
            at circles 0

        second =
            at circles 1

        a0 =
            { first | x = -second.r, y = 0 }

        a1 =
            { second | x = first.r, y = 0 }

        stage =
            circles |> Array.set 0 a0 |> Array.set 1 a1 |> Array.set 2 (place a1 a0 (at circles 2))

        linked dir a b c =
            Array.repeat n 0 |> Array.set dir a |> Array.set (modBy 3 (dir + 1)) b |> Array.set (modBy 3 (dir + 2)) c

        insert arr prev next a b i =
            if i == n then
                let
                    frontier j acc =
                        if j == b then
                            List.reverse acc

                        else
                            frontier (index next j) (at arr j :: acc)

                    front =
                        frontier (index next b) [ at arr b ]
                in
                enclose front seed |> Result.map (\( e, s ) -> ( Array.map (\v -> { v | x = v.x - e.x, y = v.y - e.y }) arr, e.r, s ))

            else
                let
                    c =
                        place (at arr a) (at arr b) (at arr i)

                    now =
                        Array.set i c arr

                    scan j k sj sk =
                        if sj <= sk then
                            if intersects (at arr j) c then
                                Just ( a, j )

                            else
                                let
                                    jj =
                                        index next j
                                in
                                if jj == index next k then
                                    Nothing

                                else
                                    scan jj k (sj + (at arr j).r) sk

                        else if intersects (at arr k) c then
                            Just ( k, b )

                        else
                            let
                                kk =
                                    index prev k
                            in
                            if j == index next kk then
                                Nothing

                            else
                                scan j kk sj (sk + (at arr k).r)

                    hit =
                        scan (index next b) (index prev a) (at arr b).r (at arr a).r
                in
                case hit of
                    Just ( aa, bb ) ->
                        insert now (Array.set bb aa prev) (Array.set aa bb next) aa bb i

                    Nothing ->
                        let
                            nx =
                                next |> Array.set i b |> Array.set a i

                            pv =
                                prev |> Array.set i a |> Array.set b i

                            closest j best bestScore =
                                if j == i then
                                    best

                                else
                                    let
                                        s =
                                            score (at now j) (at now (index nx j))
                                    in
                                    if s < bestScore then
                                        closest (index nx j) j s

                                    else
                                        closest (index nx j) best bestScore

                            left =
                                closest (index nx i) a (score (at now a) (at now i))
                        in
                        insert now pv nx left (index nx left) (i + 1)
    in
    if n == 1 then
        Ok ( Array.set 0 { first | x = 0, y = 0 } circles, first.r, seed )

    else if n == 2 then
        Ok ( stage, first.r + second.r, seed )

    else
        insert stage (linked 0 2 0 1) (linked 0 1 2 0) 0 1 3


layout pre post root w h padding ns =
    let
        leaves i old =
            let
                n =
                    get i old
            in
            if List.isEmpty n.kids then
                let
                    r =
                        sqrt n.value
                in
                alter i
                    (\a ->
                        { a
                            | r =
                                if isNaN r then
                                    0

                                else
                                    max 0 r
                        }
                    )
                    old

            else
                old

        initial =
            List.foldl leaves (alter root (\n -> { n | x = w / 2, y = h / 2 }) ns) pre

        children pad i ( old, seed ) =
            let
                kids =
                    (get i old).kids

                p =
                    if isNaN pad then
                        0

                    else
                        pad

                child k =
                    let
                        n =
                            circle (get k old)
                    in
                    if p /= 0 then
                        { n | r = n.r + p }

                    else
                        n

                assign ( arr, r, s ) =
                    let
                        put ( k, c ) =
                            alter k
                                (\n ->
                                    { n
                                        | x = c.x
                                        , y = c.y
                                        , r =
                                            if p /= 0 then
                                                c.r - p

                                            else
                                                n.r
                                    }
                                )

                        updated =
                            List.foldl put old (List.map2 Tuple.pair kids (Array.toList arr))
                    in
                    ( alter i (\n -> { n | r = r + p }) updated, s )
            in
            if List.isEmpty kids then
                Ok ( old, seed )

            else
                siblings (List.map child kids) seed |> Result.map assign

        pass pad value =
            List.foldl (\i acc -> Result.andThen (children pad i) acc) (Ok value) post

        translate ( old, _ ) =
            let
                k =
                    min w h / (2 * (get root old).r)

                apply i arr =
                    let
                        n =
                            get i arr

                        parent =
                            get n.parent arr
                    in
                    alter i
                        (\a ->
                            { a
                                | r = a.r * k
                                , x =
                                    if n.parent < 0 then
                                        a.x

                                    else
                                        parent.x + k * a.x
                                , y =
                                    if n.parent < 0 then
                                        a.y

                                    else
                                        parent.y + k * a.y
                            }
                        )
                        arr
            in
            List.foldl apply old pre
    in
    pass 0 ( initial, 1 ) |> Result.andThen (\( stage, s ) -> pass (padding * ((get root stage).r / min w h)) ( stage, s )) |> Result.map translate
