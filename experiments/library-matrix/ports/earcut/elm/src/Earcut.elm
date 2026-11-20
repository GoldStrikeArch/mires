module Earcut exposing (triangulate)

import Array
import Geometry exposing (area, equal, inTriangle, intersects)
import Holes
import Morton
import Ring exposing (addTriangle, blank, filterPoints, get, linkedList, locallyInside, remove, split)


triangulate : Geometry.Coords -> List Int -> Int -> List Int
triangulate data holes dim =
    let
        outer =
            Maybe.withDefault (Array.length data // dim) (List.head holes)

        ( root, a ) =
            linkedList data dim 0 outer True blank

        last =
            get a root
    in
    if root < 0 || last.prev == last.next then
        []

    else
        let
            ( head, arena ) =
                if List.isEmpty holes then
                    ( root, a )

                else
                    Holes.eliminate data dim holes root a

            done =
                clip (Morton.bounds data dim outer) head arena
        in
        List.reverse done.triangles


isEar : Morton.Scale -> Ring.Arena -> Ring.Node -> Ring.Node -> Ring.Node -> Bool
isEar scale arena a b c =
    let
        x0 =
            min a.x (min b.x c.x)

        x1 =
            max a.x (max b.x c.x)

        y0 =
            min a.y (min b.y c.y)

        y1 =
            max a.y (max b.y c.y)

        blocked p =
            p.x
                >= x0
                && p.x
                <= x1
                && p.y
                >= y0
                && p.y
                <= y1
                && not (equal a p)
                && inTriangle a b c p
                && area (get arena p.prev) p (get arena p.next)
                >= 0

        walk id =
            if id == a.id then
                True

            else
                let
                    p =
                        get arena id
                in
                if blocked p then
                    False

                else
                    walk p.next

        backward limit id =
            if id < 0 then
                True

            else
                let
                    p =
                        get arena id
                in
                if p.z < limit then
                    True

                else if p.id /= c.id && blocked p then
                    False

                else
                    backward limit p.prevZ

        forward limit id =
            if id < 0 then
                True

            else
                let
                    p =
                        get arena id
                in
                if p.z > limit then
                    True

                else if p.id /= c.id && blocked p then
                    False

                else
                    forward limit p.nextZ
    in
    if scale.factor == 0 then
        walk c.next

    else
        backward (Morton.zOrder scale x0 y0) b.prevZ && forward (Morton.zOrder scale x1 y1) b.nextZ


cure : Int -> Ring.Arena -> ( Int, Ring.Arena )
cure start a =
    let
        loop root id cured arena =
            let
                p =
                    get arena id

                before =
                    get arena p.prev

                next =
                    get arena p.next

                b =
                    get arena next.next

                crosses =
                    intersects False before p next b && locallyInside arena before b && locallyInside arena b before
            in
            if crosses then
                let
                    removedP =
                        arena |> addTriangle before p b |> remove p

                    removed =
                        remove (get removedP p.next) removedP

                    newNext =
                        (get removed b.id).next
                in
                if newNext == b.id then
                    let
                        ( end, healed, _ ) =
                            filterPoints b.id b.id removed
                    in
                    ( end, healed )

                else
                    loop b.id newNext True removed

            else if p.next /= root then
                loop root p.next cured arena

            else if cured then
                let
                    ( end, healed, _ ) =
                        filterPoints root root arena
                in
                ( end, healed )

            else
                ( root, arena )
    in
    loop start start False a


validDiagonal : Ring.Arena -> Ring.Node -> Ring.Node -> Bool
validDiagonal a p q =
    let
        pp =
            get a p.prev

        pn =
            get a p.next

        qp =
            get a q.prev

        qn =
            get a q.next

        zero =
            equal p q && area pp p pn > 0 && area qp q qn > 0

        crossing id =
            let
                v =
                    get a id

                n =
                    get a v.next

                separate h j =
                    (v.x > h && n.x > h) || (v.x < j && n.x < j)

                outside =
                    separate (max p.x q.x) (min p.x q.x)
                        || (v.y > max p.y q.y && n.y > max p.y q.y)
                        || (v.y < min p.y q.y && n.y < min p.y q.y)
            in
            if not outside && v.i /= p.i && n.i /= p.i && v.i /= q.i && n.i /= q.i && intersects True v n p q then
                True

            else if v.next == p.id then
                False

            else
                crossing v.next

        midX =
            (p.x + q.x) / 2

        midY =
            (p.y + q.y) / 2

        inside id state =
            let
                v =
                    get a id

                n =
                    get a v.next

                toggles =
                    (v.y > midY) /= (n.y > midY) && midX < (n.x - v.x) * (midY - v.y) / (n.y - v.y) + v.x

                result =
                    if toggles then
                        not state

                    else
                        state
            in
            if v.next == p.id then
                result

            else
                inside v.next result
    in
    pn.i
        /= q.i
        && (zero
                || (locallyInside a p q
                        && locallyInside a q p
                        && (area pp p qp /= 0 || area p qp q /= 0)
                   )
           )
        && not (crossing p.id)
        && (zero || inside p.id False)


findDiagonal : Int -> Ring.Arena -> Maybe ( Int, Int )
findDiagonal start a =
    let
        from id =
            let
                p =
                    get a id

                try bid =
                    if bid == p.prev then
                        Nothing

                    else
                        let
                            q =
                                get a bid
                        in
                        if p.i /= q.i && validDiagonal a p q then
                            Just ( id, bid )

                        else
                            try q.next
            in
            case try (get a p.next).next of
                Just diagonal ->
                    Just diagonal

                Nothing ->
                    if p.next == start then
                        Nothing

                    else
                        from p.next
    in
    from start


clip : Morton.Scale -> Int -> Ring.Arena -> Ring.Arena
clip scale first a =
    let
        loop id stop cured arena =
            let
                p =
                    get arena id

                before =
                    get arena p.prev

                next =
                    get arena p.next
            in
            if p.prev == p.next then
                arena

            else if area before p next < 0 && isEar scale arena before p next then
                loop p.next p.next cured (arena |> addTriangle before p next |> remove p)

            else if p.next /= stop then
                loop p.next stop cured arena

            else
                let
                    ( head, filtered, removed ) =
                        filterPoints stop stop arena
                in
                if removed then
                    loop head head cured filtered

                else if not cured then
                    let
                        ( healed, healedArena ) =
                            cure head filtered
                    in
                    loop healed healed True healedArena

                else
                    case findDiagonal head filtered of
                        Nothing ->
                            filtered

                        Just ( ai, bi ) ->
                            let
                                ( ci, divided ) =
                                    split ai bi filtered

                                ( left, leftHealed, _ ) =
                                    filterPoints ai (get divided ai).next divided

                                ( right, bothHealed, _ ) =
                                    filterPoints ci (get leftHealed ci).next leftHealed
                            in
                            clip scale right (clip scale left bothHealed)
    in
    loop first
        first
        False
        (if scale.factor == 0 then
            a

         else
            Morton.indexCurve scale first a
        )
