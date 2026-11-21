module Rectangles exposing (layout)

import Arena exposing (alter, get)
import Array
import JsMath exposing (jsMax, truthy)


type alias Rect =
    { x : Float, y : Float, xx : Float, yy : Float }


put r n =
    { n | x0 = r.x, y0 = r.y, x1 = r.xx, y1 = r.yy }


collapse r =
    let
        a =
            if r.xx < r.x then
                { r | x = (r.x + r.xx) / 2, xx = (r.x + r.xx) / 2 }

            else
                r
    in
    if a.yy < a.y then
        { a | y = (a.y + a.yy) / 2, yy = (a.y + a.yy) / 2 }

    else
        a


inset p r =
    collapse (Rect (r.x + p) (r.y + p) (r.xx - p) (r.yy - p))


getAt i a =
    Array.get i a |> Maybe.withDefault 0


slice kids total r dice ns =
    let
        scale =
            if not (truthy total) then
                total

            else if dice then
                (r.xx - r.x) / total

            else
                (r.yy - r.y) / total

        step i ( pos, old ) =
            let
                end =
                    pos + (get i old).value * scale

                rect =
                    if dice then
                        { r | x = pos, xx = end }

                    else
                        { r | y = pos, yy = end }
            in
            ( end, alter i (put rect) old )
    in
    List.foldl step
        ( if dice then
            r.x

          else
            r.y
        , ns
        )
        kids
        |> Tuple.second


binary kids value frame ns =
    let
        ids =
            Array.fromList kids

        sums =
            List.foldl (\i acc -> ((get i ns).value + Maybe.withDefault 0 (List.head acc)) :: acc) [ 0 ] kids |> List.reverse |> Array.fromList

        split i j total r old =
            if i >= j - 1 then
                alter (getAt i ids) (put r) old

            else
                let
                    off =
                        getAt i sums

                    target =
                        total / 2 + off

                    search lower hi =
                        if lower >= hi then
                            lower

                        else
                            let
                                mid =
                                    (lower + hi) // 2
                            in
                            if getAt mid sums < target then
                                search (mid + 1) hi

                            else
                                search lower mid

                    pivot =
                        search (i + 1) (j - 1)

                    k =
                        if target - getAt (pivot - 1) sums < getAt pivot sums - target && i + 1 < pivot then
                            pivot - 1

                        else
                            pivot

                    left =
                        getAt k sums - off

                    right =
                        total - left
                in
                if r.yy - r.y < r.xx - r.x then
                    let
                        x =
                            if truthy total then
                                (r.x * right + r.xx * left) / total

                            else
                                r.xx
                    in
                    split k j right { r | x = x } (split i k left { r | xx = x } old)

                else
                    let
                        y =
                            if truthy total then
                                (r.y * right + r.yy * left) / total

                            else
                                r.yy
                    in
                    split k j right { r | y = y } (split i k left { r | yy = y } old)
    in
    split 0 (Array.length ids) value frame ns


squarify kids total frame ns =
    let
        ratio =
            (1 + sqrt 5) / 2

        nextNonZero left acc =
            case left of
                [] ->
                    ( [], acc, 0 )

                i :: rest ->
                    if truthy (get i ns).value || List.isEmpty rest then
                        ( rest, i :: acc, (get i ns).value )

                    else
                        nextNonZero rest (i :: acc)

        rows remaining value r old =
            if List.isEmpty remaining then
                old

            else
                let
                    dx =
                        r.xx - r.x

                    dy =
                        r.yy - r.y

                    ( rest, initial, first ) =
                        nextNonZero remaining []

                    alpha =
                        jsMax (dy / dx) (dx / dy) / (value * ratio)

                    beta =
                        first * first * alpha

                    collect left built sumValue lo hi best =
                        case left of
                            [] ->
                                ( [], List.reverse built, sumValue )

                            i :: tail ->
                                let
                                    v =
                                        (get i old).value

                                    candidateSum =
                                        sumValue + v

                                    lower =
                                        if v < lo then
                                            v

                                        else
                                            lo

                                    upper =
                                        if hi < v then
                                            v

                                        else
                                            hi

                                    b =
                                        candidateSum * candidateSum * alpha

                                    candidate =
                                        jsMax (upper / b) (b / lower)
                                in
                                if best < candidate then
                                    ( left, List.reverse built, candidateSum - v )

                                else
                                    collect tail (i :: built) candidateSum lower upper candidate

                    ( pending, row, sum ) =
                        collect rest initial first first first (jsMax (first / beta) (beta / first))
                in
                if dx < dy then
                    let
                        bottom =
                            if truthy value then
                                r.y + dy * sum / value

                            else
                                r.yy
                    in
                    rows pending
                        (value - sum)
                        (if truthy value then
                            { r | y = bottom }

                         else
                            r
                        )
                        (slice row sum { r | yy = bottom } True old)

                else
                    let
                        right =
                            if truthy value then
                                r.x + dx * sum / value

                            else
                                r.xx
                    in
                    rows pending
                        (value - sum)
                        (if truthy value then
                            { r | x = right }

                         else
                            r
                        )
                        (slice row sum { r | xx = right } False old)
    in
    rows kids total frame ns


layout pre root w h p partition tile rounded ns =
    let
        levels =
            toFloat ((get root ns).height + 1)

        initial =
            if partition then
                Rect p p w (h / levels)

            else
                Rect 0 0 w h

        step i old =
            let
                n =
                    get i old

                r =
                    Rect n.x0 n.y0 n.x1 n.y1
            in
            if partition then
                let
                    child =
                        Rect r.x (h * toFloat (n.depth + 1) / levels) r.xx (h * toFloat (n.depth + 2) / levels)

                    laid =
                        slice n.kids n.value child True old
                in
                alter i (put (collapse { r | xx = r.xx - p, yy = r.yy - p })) laid

            else
                let
                    inner =
                        inset
                            (if i == root then
                                0

                             else
                                p / 2
                            )
                            r

                    placed =
                        alter i (put inner) old

                    frame =
                        inset (p - p / 2) inner
                in
                if List.isEmpty n.kids then
                    placed

                else if tile == "binary" then
                    binary n.kids n.value frame placed

                else if tile == "sliceDice" then
                    slice n.kids n.value frame (modBy 2 n.depth == 0) placed

                else
                    squarify n.kids n.value frame placed

        result =
            List.foldl step (alter root (put initial) ns) pre

        nearest x =
            toFloat (round x)
    in
    if rounded then
        List.foldl (\i -> alter i (\n -> { n | x0 = nearest n.x0, x1 = nearest n.x1, y0 = nearest n.y0, y1 = nearest n.y1 })) result pre

    else
        result
